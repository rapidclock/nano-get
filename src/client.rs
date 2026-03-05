use std::collections::HashMap;
use std::io::BufReader;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use crate::auth::{
    parse_authenticate_headers, AuthDecision, AuthHandler, AuthTarget, BasicAuthHandler, Challenge,
    DynAuthHandler,
};
use crate::date::parse_http_date;
use crate::errors::NanoGetError;
use crate::http::{self, BoxStream};
#[cfg(feature = "https")]
use crate::https;
use crate::request::{should_follow_redirect, Header, Method, RedirectPolicy, Request};
use crate::response::Response;
use crate::url::{ToUrl, Url};

/// Controls whether requests use one-off sockets or reusable persistent connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionPolicy {
    /// Open a fresh connection per request and close it after each response.
    Close,
    /// Reuse compatible persistent connections when possible.
    Reuse,
}

/// Controls whether the built-in in-memory cache is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheMode {
    /// Disable caching entirely.
    Disabled,
    /// Enable the built-in process-local memory cache.
    Memory,
}

/// Controls how strictly incoming HTTP/1.1 responses are parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParserStrictness {
    /// Fail closed on ambiguous or unsafe wire framing.
    Strict,
    /// Accept a compatibility-oriented subset of non-strict server behavior.
    Lenient,
}

impl ParserStrictness {
    const fn is_strict(self) -> bool {
        matches!(self, Self::Strict)
    }
}

/// Proxy configuration for routing requests through an HTTP proxy.
///
/// The proxy URL itself must be `http://...`, including when tunneling HTTPS via `CONNECT`.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    url: Url,
    headers: Vec<Header>,
}

impl ProxyConfig {
    /// Creates a new proxy configuration.
    ///
    /// The URL must use `http://`; otherwise [`NanoGetError::UnsupportedProxyScheme`] is
    /// returned.
    pub fn new<U: ToUrl>(url: U) -> Result<Self, NanoGetError> {
        let url = url.to_url()?;
        if !url.is_http() {
            return Err(NanoGetError::UnsupportedProxyScheme(url.scheme.clone()));
        }

        Ok(Self {
            url,
            headers: Vec::new(),
        })
    }

    /// Returns the parsed proxy URL.
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// Returns additional headers that will be attached to proxy requests.
    pub fn headers(&self) -> &[Header] {
        &self.headers
    }

    /// Adds a custom header to outgoing proxy requests.
    ///
    /// Protocol-managed and hop-by-hop headers are rejected.
    pub fn add_header(
        &mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<&mut Self, NanoGetError> {
        let name = name.into();
        validate_proxy_header_name(&name)?;
        self.headers.push(Header::new(name, value)?);
        Ok(self)
    }
}

/// Builder for configuring a [`Client`].
#[derive(Clone)]
pub struct ClientBuilder {
    config: ClientConfig,
}

impl ClientBuilder {
    /// Creates a new builder with release-default settings.
    pub fn new() -> Self {
        Self {
            config: ClientConfig::default(),
        }
    }

    /// Sets the default redirect policy used by `Client::get`, `Client::head`, and requests
    /// that do not override redirect handling explicitly.
    pub fn redirect_policy(mut self, policy: RedirectPolicy) -> Self {
        self.config.redirect_policy = policy;
        self
    }

    /// Configures whether sessions should close each connection after one request or reuse
    /// compatible connections for sequential and pipelined traffic.
    pub fn connection_policy(mut self, policy: ConnectionPolicy) -> Self {
        self.config.connection_policy = policy;
        self
    }

    /// Enables or disables the built-in in-memory cache.
    pub fn cache_mode(mut self, mode: CacheMode) -> Self {
        self.config.cache_mode = mode;
        self
    }

    /// Configures how strictly response framing and line syntax are validated.
    ///
    /// The default is [`ParserStrictness::Strict`].
    pub fn parser_strictness(mut self, strictness: ParserStrictness) -> Self {
        self.config.parser_strictness = strictness;
        self
    }

    /// Routes requests through an explicit HTTP proxy.
    pub fn proxy(mut self, proxy: ProxyConfig) -> Self {
        self.config.proxy = Some(proxy);
        self
    }

    /// Installs a generic origin-authentication handler for `401` challenges.
    pub fn auth_handler(mut self, handler: Arc<dyn AuthHandler + Send + Sync>) -> Self {
        self.config.auth_handler = Some(handler);
        self
    }

    /// Installs a generic proxy-authentication handler for `407` challenges.
    pub fn proxy_auth_handler(mut self, handler: Arc<dyn AuthHandler + Send + Sync>) -> Self {
        self.config.proxy_auth_handler = Some(handler);
        self
    }

    /// Enables challenge-driven HTTP Basic authentication for origin servers.
    pub fn basic_auth(mut self, username: impl Into<String>, password: impl Into<String>) -> Self {
        let handler = Arc::new(BasicAuthHandler::new(
            username.into(),
            password.into(),
            AuthTarget::Origin,
        ));
        self.config.auth_handler = Some(handler);
        self
    }

    /// Enables challenge-driven HTTP Basic authentication for the configured proxy.
    pub fn basic_proxy_auth(
        mut self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        let handler = Arc::new(BasicAuthHandler::new(
            username.into(),
            password.into(),
            AuthTarget::Proxy,
        ));
        self.config.proxy_auth_handler = Some(handler);
        self
    }

    /// Sends HTTP Basic origin credentials on the first request and still handles later `401`
    /// challenges with the same credentials.
    pub fn preemptive_basic_auth(
        mut self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        let handler = BasicAuthHandler::new(username.into(), password.into(), AuthTarget::Origin);
        self.config.preemptive_authorization = Some(handler.header_value().to_string());
        self.config.auth_handler = Some(Arc::new(handler));
        self
    }

    /// Sends HTTP Basic proxy credentials on the first proxy request or `CONNECT` attempt and
    /// still handles later `407` challenges with the same credentials.
    pub fn preemptive_basic_proxy_auth(
        mut self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        let handler = BasicAuthHandler::new(username.into(), password.into(), AuthTarget::Proxy);
        self.config.preemptive_proxy_authorization = Some(handler.header_value().to_string());
        self.config.proxy_auth_handler = Some(Arc::new(handler));
        self
    }

    /// Builds a [`Client`] from the current builder configuration.
    pub fn build(self) -> Client {
        Client {
            inner: Arc::new(ClientInner {
                config: self.config,
                cache: Arc::new(Mutex::new(MemoryCache::default())),
            }),
        }
    }
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Reusable synchronous HTTP client.
///
/// Use this when you want to configure redirects, connection reuse, caching, proxy behavior,
/// and authentication once, then execute many requests.
#[derive(Clone)]
pub struct Client {
    inner: Arc<ClientInner>,
}

struct ClientInner {
    config: ClientConfig,
    cache: Arc<Mutex<MemoryCache>>,
}

impl Client {
    /// Creates a new [`ClientBuilder`].
    pub fn builder() -> ClientBuilder {
        ClientBuilder::new()
    }

    /// Creates a [`Session`] that can reuse a live connection for sequential requests.
    pub fn session(&self) -> Session {
        Session {
            config: self.inner.config.clone(),
            cache: Arc::clone(&self.inner.cache),
            connection: None,
        }
    }

    /// Executes a fully configured [`Request`].
    pub fn execute(&self, request: Request) -> Result<Response, NanoGetError> {
        self.session().execute(request)
    }

    /// Executes a borrowed [`Request`] by cloning it internally.
    pub fn execute_ref(&self, request: &Request) -> Result<Response, NanoGetError> {
        self.execute(request.clone())
    }

    /// Performs a `GET` with the client's default redirect policy and returns UTF-8 text.
    pub fn get<U: ToUrl>(&self, url: U) -> Result<String, NanoGetError> {
        let request = Request::get(url)?.with_redirect_policy(self.inner.config.redirect_policy);
        self.execute(request).and_then(Response::into_body_text)
    }

    /// Performs a `GET` with the client's default redirect policy and returns raw bytes.
    pub fn get_bytes<U: ToUrl>(&self, url: U) -> Result<Vec<u8>, NanoGetError> {
        let request = Request::get(url)?.with_redirect_policy(self.inner.config.redirect_policy);
        self.execute(request).map(|response| response.body)
    }

    /// Performs a `HEAD` with the client's default redirect policy.
    pub fn head<U: ToUrl>(&self, url: U) -> Result<Response, NanoGetError> {
        let request = Request::head(url)?.with_redirect_policy(self.inner.config.redirect_policy);
        self.execute(request)
    }
}

impl Default for Client {
    fn default() -> Self {
        ClientBuilder::default().build()
    }
}

/// Stateful request executor that can hold a persistent connection.
///
/// A session is useful when you want tighter control over connection reuse and pipelining than
/// calling [`Client::execute`] repeatedly.
pub struct Session {
    config: ClientConfig,
    cache: Arc<Mutex<MemoryCache>>,
    connection: Option<LiveConnection>,
}

impl Session {
    /// Executes a request, applying configured redirects, cache behavior, auth retries,
    /// and proxy routing.
    pub fn execute(&mut self, request: Request) -> Result<Response, NanoGetError> {
        let redirect_policy = request.effective_redirect_policy(self.config.redirect_policy);
        let mut current = request;
        let mut followed = 0usize;

        loop {
            let response = self.execute_one(current.clone())?;

            match redirect_policy {
                RedirectPolicy::None => return Ok(response),
                RedirectPolicy::Follow { max_redirects } => {
                    if !should_follow_redirect(response.status_code) {
                        return Ok(response);
                    }

                    let Some(location) = response.header("location") else {
                        return Ok(response);
                    };

                    if followed >= max_redirects {
                        return Err(NanoGetError::RedirectLimitExceeded(max_redirects));
                    }

                    let next_url = current.url().resolve(location)?;
                    let same_authority = current.url().same_authority(&next_url);
                    current = current.clone_with_url(next_url);
                    if !same_authority {
                        current.remove_headers_named("authorization");
                        current.disable_preemptive_origin_auth();
                    }
                    followed += 1;
                }
            }
        }
    }

    /// Executes a borrowed request by cloning it internally.
    pub fn execute_ref(&mut self, request: &Request) -> Result<Response, NanoGetError> {
        self.execute(request.clone())
    }

    /// Sends multiple requests in one HTTP/1.1 pipeline.
    ///
    /// Requirements:
    /// - [`ConnectionPolicy::Reuse`] must be enabled
    /// - all requests must target the same underlying connection
    ///
    /// On success, responses are returned in request order.
    pub fn execute_pipelined(
        &mut self,
        requests: &[Request],
    ) -> Result<Vec<Response>, NanoGetError> {
        if requests.is_empty() {
            return Ok(Vec::new());
        }

        for request in requests {
            validate_request_conditionals(request)?;
        }

        if self.config.connection_policy == ConnectionPolicy::Close {
            return Err(NanoGetError::Pipeline(
                "pipelining requires ConnectionPolicy::Reuse".to_string(),
            ));
        }

        let key = connection_key(&self.config.proxy, requests[0].url());
        for request in requests {
            if request.url().scheme == "https" && self.config.proxy.is_none() {
                if connection_key(&self.config.proxy, request.url()) != key {
                    return Err(NanoGetError::Pipeline(
                        "all pipelined requests must target the same connection".to_string(),
                    ));
                }
            } else if connection_key(&self.config.proxy, request.url()) != key {
                return Err(NanoGetError::Pipeline(
                    "all pipelined requests must share the same connection".to_string(),
                ));
            }
        }

        self.ensure_connection(&requests[0])?;
        {
            let connection = self
                .connection
                .as_mut()
                .ok_or_else(|| NanoGetError::Pipeline("missing live connection".to_string()))?;

            for request in requests {
                let send_target = SendTarget::for_request(&self.config, request);
                let prepared = prepared_request(request, &self.config, send_target)?;
                let target = request_target(&prepared, &self.config.proxy);
                http::write_request(connection.reader.get_mut(), &prepared, &target, false)?;
            }
            use std::io::Write;
            connection.reader.get_mut().flush()?;
        }

        let mut responses = Vec::with_capacity(requests.len());
        for (index, request) in requests.iter().enumerate() {
            let parsed = {
                let connection = self
                    .connection
                    .as_mut()
                    .ok_or_else(|| NanoGetError::Pipeline("missing live connection".to_string()))?;
                crate::response::read_parsed_response(
                    &mut connection.reader,
                    request.method(),
                    self.config.parser_strictness.is_strict(),
                )?
            };
            responses.push(parsed.response);
            if parsed.connection_close && index + 1 != requests.len() {
                self.connection = None;
                for remaining in requests.iter().skip(index + 1) {
                    responses.push(self.execute_one(remaining.clone())?);
                }
                return Ok(responses);
            }
            if parsed.connection_close {
                self.connection = None;
            }
        }

        Ok(responses)
    }

    fn execute_one(&mut self, request: Request) -> Result<Response, NanoGetError> {
        validate_request_conditionals(&request)?;
        let auth_context = effective_auth_context(&request, &self.config);
        let cache_directives = CacheControl::from_headers(request.headers());
        if self.config.cache_mode != CacheMode::Memory && cache_directives.only_if_cached {
            return Ok(gateway_timeout_response());
        }

        let now = SystemTime::now();
        let mut bypass_standard_cache = false;
        if self.config.cache_mode == CacheMode::Memory
            && request.method() == Method::Get
            && request.has_header("range")
            && !cache_directives.no_store
        {
            let range_lookup = self
                .cache
                .lock()
                .map_err(|_| NanoGetError::Cache("cache lock poisoned".to_string()))?
                .lookup_range(&request, now, &auth_context);
            match range_lookup {
                Some(RangeCacheLookup::Hit(response)) => {
                    return Ok(response_for_method(&response, request.method()))
                }
                Some(RangeCacheLookup::UnsatisfiedOnlyIfCached) => {
                    return Ok(gateway_timeout_response())
                }
                Some(RangeCacheLookup::IfRangeMismatch) => {
                    bypass_standard_cache = true;
                }
                None => {}
            }
        }

        let cache_lookup = if !bypass_standard_cache
            && self.config.cache_mode == CacheMode::Memory
            && request.method() == Method::Get
            && !cache_directives.no_store
        {
            self.cache
                .lock()
                .map_err(|_| NanoGetError::Cache("cache lock poisoned".to_string()))?
                .lookup(&request, now, &auth_context)
        } else {
            None
        };

        match cache_lookup {
            Some(CacheLookup::Fresh(entry)) => {
                return Ok(response_for_method(
                    &entry.response_with_age(now),
                    request.method(),
                ))
            }
            Some(CacheLookup::Stale(entry)) => {
                let revalidation = self.execute_stale(request, *entry)?;
                return Ok(revalidation);
            }
            Some(CacheLookup::UnsatisfiedOnlyIfCached) => return Ok(gateway_timeout_response()),
            None => {}
        }

        let mut current = request;
        let mut seen_origin_challenges = Vec::new();
        let mut seen_proxy_challenges = Vec::new();

        loop {
            let timed_response = self.send_request(&current)?;
            let response = timed_response.response.clone();

            if response.status_code == 401 {
                if let Some(next_request) = self.maybe_retry_auth(
                    AuthTarget::Origin,
                    &current,
                    &response,
                    &mut seen_origin_challenges,
                )? {
                    current = next_request;
                    continue;
                }
            } else if response.status_code == 407 {
                if let Some(next_request) = self.maybe_retry_auth(
                    AuthTarget::Proxy,
                    &current,
                    &response,
                    &mut seen_proxy_challenges,
                )? {
                    current = next_request;
                    continue;
                }
            }

            let final_auth_context = effective_auth_context(&current, &self.config);
            self.store_in_cache(&current, &timed_response, &final_auth_context)?;
            return Ok(response);
        }
    }

    fn execute_stale(
        &mut self,
        request: Request,
        entry: CacheEntry,
    ) -> Result<Response, NanoGetError> {
        let mut conditional_request = request.clone();
        if !has_user_conditionals(&conditional_request) {
            if let Some(etag) = &entry.etag {
                conditional_request.if_none_match(etag.clone())?;
            } else if let Some(last_modified) = &entry.last_modified {
                conditional_request.set_header("If-Modified-Since", last_modified.clone())?;
            }
        }

        let response = self.send_request(&conditional_request)?.response;
        if response.status_code == 304 {
            let merged = {
                let mut cache = self
                    .cache
                    .lock()
                    .map_err(|_| NanoGetError::Cache("cache lock poisoned".to_string()))?;
                cache.merge_not_modified(&request, &entry, &response, SystemTime::now())?
            };
            return Ok(response_for_method(&merged, request.method()));
        }

        let timed_response = TimedResponse::synthetic(response.clone());
        let auth_context = effective_auth_context(&request, &self.config);
        self.store_in_cache(&request, &timed_response, &auth_context)?;
        Ok(response)
    }

    fn send_request(&mut self, request: &Request) -> Result<TimedResponse, NanoGetError> {
        let should_reuse = self.config.connection_policy == ConnectionPolicy::Reuse;
        let mut retried = false;

        loop {
            let result = if should_reuse {
                self.ensure_connection(request)?;
                self.send_on_live_connection(request)
            } else {
                self.send_ephemeral(request)
            };

            match result {
                Ok(response) => return Ok(response),
                Err(error) if should_reuse && !retried => {
                    self.connection = None;
                    retried = true;
                    if matches!(
                        error,
                        NanoGetError::Io(_) | NanoGetError::Connect(_) | NanoGetError::Tls(_)
                    ) {
                        continue;
                    }
                    return Err(error);
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn send_ephemeral(&self, request: &Request) -> Result<TimedResponse, NanoGetError> {
        let mut connection = open_connection(&self.config, request)?;
        let prepared = prepared_request(
            request,
            &self.config,
            SendTarget::for_request(&self.config, request),
        )?;
        let target = request_target(&prepared, &self.config.proxy);
        use std::io::Write;

        let request_time = SystemTime::now();
        http::write_request(connection.reader.get_mut(), &prepared, &target, true)?;
        connection.reader.get_mut().flush()?;
        let parsed = crate::response::read_parsed_response(
            &mut connection.reader,
            request.method(),
            self.config.parser_strictness.is_strict(),
        )?;
        let response_time = SystemTime::now();
        Ok(TimedResponse {
            response: parsed.response,
            request_time,
            response_time,
        })
    }

    fn send_on_live_connection(
        &mut self,
        request: &Request,
    ) -> Result<TimedResponse, NanoGetError> {
        let send_target = SendTarget::for_request(&self.config, request);
        let prepared = prepared_request(request, &self.config, send_target)?;
        let target = request_target(&prepared, &self.config.proxy);
        let connection = self.connection.as_mut().ok_or_else(|| {
            NanoGetError::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "missing persistent connection",
            ))
        })?;
        use std::io::Write;

        let request_time = SystemTime::now();
        http::write_request(connection.reader.get_mut(), &prepared, &target, false)?;
        connection.reader.get_mut().flush()?;
        let parsed = crate::response::read_parsed_response(
            &mut connection.reader,
            request.method(),
            self.config.parser_strictness.is_strict(),
        )?;
        let response_time = SystemTime::now();
        let response = parsed.response;
        if parsed.connection_close {
            self.connection = None;
        }
        Ok(TimedResponse {
            response,
            request_time,
            response_time,
        })
    }

    fn ensure_connection(&mut self, request: &Request) -> Result<(), NanoGetError> {
        let desired = connection_key(&self.config.proxy, request.url());
        let keep_existing = self
            .connection
            .as_ref()
            .map(|connection| connection.key == desired)
            .unwrap_or(false);
        if keep_existing {
            return Ok(());
        }

        self.connection = Some(open_connection(&self.config, request)?);
        Ok(())
    }

    fn store_in_cache(
        &self,
        request: &Request,
        timed_response: &TimedResponse,
        auth_context: &AuthContext,
    ) -> Result<(), NanoGetError> {
        if self.config.cache_mode != CacheMode::Memory
            || !matches!(request.method(), Method::Get | Method::Head)
            || CacheControl::from_headers(request.headers()).no_store
        {
            return Ok(());
        }

        let mut cache = self
            .cache
            .lock()
            .map_err(|_| NanoGetError::Cache("cache lock poisoned".to_string()))?;
        cache.store(request, timed_response, auth_context);
        Ok(())
    }

    fn maybe_retry_auth(
        &self,
        target: AuthTarget,
        request: &Request,
        response: &Response,
        seen_challenges: &mut Vec<Vec<Challenge>>,
    ) -> Result<Option<Request>, NanoGetError> {
        let handler = match target {
            AuthTarget::Origin => self.config.auth_handler.as_ref(),
            AuthTarget::Proxy => self.config.proxy_auth_handler.as_ref(),
        };
        maybe_retry_request_auth(handler, target, request, response, seen_challenges)
    }
}

#[derive(Clone)]
struct ClientConfig {
    redirect_policy: RedirectPolicy,
    connection_policy: ConnectionPolicy,
    cache_mode: CacheMode,
    parser_strictness: ParserStrictness,
    proxy: Option<ProxyConfig>,
    auth_handler: Option<DynAuthHandler>,
    proxy_auth_handler: Option<DynAuthHandler>,
    preemptive_authorization: Option<String>,
    preemptive_proxy_authorization: Option<String>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            redirect_policy: RedirectPolicy::none(),
            connection_policy: ConnectionPolicy::Close,
            cache_mode: CacheMode::Disabled,
            parser_strictness: ParserStrictness::Strict,
            proxy: None,
            auth_handler: None,
            proxy_auth_handler: None,
            preemptive_authorization: None,
            preemptive_proxy_authorization: None,
        }
    }
}

struct LiveConnection {
    key: ConnectionKey,
    reader: BufReader<BoxStream>,
}

#[derive(Debug, Clone)]
struct TimedResponse {
    response: Response,
    request_time: SystemTime,
    response_time: SystemTime,
}

impl TimedResponse {
    fn synthetic(response: Response) -> Self {
        let now = SystemTime::now();
        Self {
            response,
            request_time: now,
            response_time: now,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SendTarget {
    Direct,
    HttpProxy,
    Tunnel,
}

impl SendTarget {
    fn for_request(config: &ClientConfig, request: &Request) -> Self {
        match config.proxy {
            Some(_) if request.url().is_http() => Self::HttpProxy,
            Some(_) => Self::Tunnel,
            None => Self::Direct,
        }
    }

    fn uses_proxy(self) -> bool {
        matches!(self, Self::HttpProxy)
    }

    fn allows_origin_auth(self) -> bool {
        true
    }

    fn allows_proxy_auth(self) -> bool {
        matches!(self, Self::HttpProxy)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct AuthContext {
    origin: Option<String>,
    proxy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConnectionKey {
    Direct { scheme: String, authority: String },
    HttpProxy { proxy: String },
    HttpsTunnel { proxy: String, target: String },
}

fn connection_key(proxy: &Option<ProxyConfig>, url: &Url) -> ConnectionKey {
    match proxy {
        Some(proxy) if url.is_http() => ConnectionKey::HttpProxy {
            proxy: proxy.url().authority_form(),
        },
        Some(proxy) => ConnectionKey::HttpsTunnel {
            proxy: proxy.url().authority_form(),
            target: url.authority_form(),
        },
        None => ConnectionKey::Direct {
            scheme: url.scheme.clone(),
            authority: url.authority_form(),
        },
    }
}

fn prepared_request(
    request: &Request,
    config: &ClientConfig,
    send_target: SendTarget,
) -> Result<Request, NanoGetError> {
    let mut prepared = request.clone();

    if !send_target.allows_proxy_auth() {
        prepared.remove_headers_named("proxy-authorization");
    }

    if send_target.uses_proxy() {
        for header in config.proxy.iter().flat_map(|proxy| proxy.headers()) {
            if prepared.has_header(header.name()) {
                continue;
            }
            prepared.add_header(header.name().to_string(), header.value().to_string())?;
        }
    }
    if send_target.allows_origin_auth()
        && prepared.preemptive_origin_auth_allowed()
        && !prepared.has_header("authorization")
    {
        if let Some(value) = &config.preemptive_authorization {
            prepared.authorization(value.clone())?;
        }
    }

    if send_target.allows_proxy_auth() && !prepared.has_header("proxy-authorization") {
        if let Some(value) = &config.preemptive_proxy_authorization {
            prepared.proxy_authorization(value.clone())?;
        }
    }

    Ok(prepared)
}

fn request_target(request: &Request, proxy: &Option<ProxyConfig>) -> String {
    if proxy.is_some() && request.url().is_http() {
        request.url().absolute_form()
    } else {
        request.url().origin_form()
    }
}

fn open_connection(
    config: &ClientConfig,
    request: &Request,
) -> Result<LiveConnection, NanoGetError> {
    let key = connection_key(&config.proxy, request.url());
    let stream = match &config.proxy {
        Some(proxy) if request.url().is_http() => {
            let stream = http::connect_tcp(&proxy.url().authority_form())?;
            Box::new(stream) as BoxStream
        }
        Some(proxy) => open_https_tunnel(config, request, proxy)?,
        None if request.url().is_http() => {
            let stream = http::connect_tcp(&request.url().authority_form())?;
            Box::new(stream) as BoxStream
        }
        None if request.url().is_https() => {
            #[cfg(feature = "https")]
            {
                https::connect_tls(request.url())?
            }
            #[cfg(not(feature = "https"))]
            {
                return Err(NanoGetError::HttpsFeatureRequired);
            }
        }
        None => {
            return Err(NanoGetError::UnsupportedScheme(
                request.url().scheme.clone(),
            ))
        }
    };

    Ok(LiveConnection {
        key,
        reader: BufReader::new(stream),
    })
}

fn open_https_tunnel(
    config: &ClientConfig,
    request: &Request,
    proxy: &ProxyConfig,
) -> Result<BoxStream, NanoGetError> {
    let mut current = request.clone();
    let mut seen_proxy_challenges = Vec::new();

    loop {
        let mut stream = http::connect_tcp(&proxy.url().authority_form())?;
        let connect_headers = prepared_connect_headers(&current, config, proxy)?;
        let authority = request.url().authority_form();
        http::write_connect_request(&mut stream, &authority, &connect_headers, false)?;
        use std::io::Write;
        stream.flush()?;
        let head = http::read_response_head(&mut stream, config.parser_strictness.is_strict())?;
        if (200..=299).contains(&head.status_code) {
            #[cfg(feature = "https")]
            {
                return https::connect_tls_over_stream(request.url(), stream);
            }
            #[cfg(not(feature = "https"))]
            {
                let _ = stream;
                return Err(NanoGetError::HttpsFeatureRequired);
            }
        }

        if head.status_code != 407 {
            break Err(NanoGetError::ProxyConnectFailed(
                head.status_code,
                head.reason_phrase,
            ));
        }
        let response = Response {
            version: head.version,
            status_code: head.status_code,
            reason_phrase: head.reason_phrase.clone(),
            headers: head.headers,
            trailers: Vec::new(),
            body: Vec::new(),
        };
        let retry = maybe_retry_request_auth(
            config.proxy_auth_handler.as_ref(),
            AuthTarget::Proxy,
            &current,
            &response,
            &mut seen_proxy_challenges,
        )?;
        if let Some(retry) = retry {
            current = retry;
            continue;
        }
        break Err(NanoGetError::ProxyConnectFailed(
            response.status_code,
            response.reason_phrase,
        ));
    }
}

#[derive(Default)]
struct MemoryCache {
    entries: HashMap<String, Vec<CacheEntry>>,
    partial_entries: HashMap<String, Vec<PartialCacheEntry>>,
}

impl MemoryCache {
    fn lookup(
        &self,
        request: &Request,
        now: SystemTime,
        auth_context: &AuthContext,
    ) -> Option<CacheLookup> {
        let request_cache_control = CacheControl::from_headers(request.headers());
        let Some(entries) = self.entries.get(&request.url().cache_key()) else {
            return if request_cache_control.only_if_cached {
                Some(CacheLookup::UnsatisfiedOnlyIfCached)
            } else {
                None
            };
        };
        let Some(entry) = entries
            .iter()
            .filter(|entry| entry.matches(request, auth_context))
            .max_by_key(|entry| entry.response_time)
            .cloned()
        else {
            return if request_cache_control.only_if_cached {
                Some(CacheLookup::UnsatisfiedOnlyIfCached)
            } else {
                None
            };
        };

        if !entry.satisfies_request(&request_cache_control, now) {
            return if request_cache_control.only_if_cached {
                Some(CacheLookup::UnsatisfiedOnlyIfCached)
            } else {
                Some(CacheLookup::Stale(Box::new(entry)))
            };
        }

        Some(CacheLookup::Fresh(Box::new(entry)))
    }

    fn lookup_range(
        &self,
        request: &Request,
        now: SystemTime,
        auth_context: &AuthContext,
    ) -> Option<RangeCacheLookup> {
        let range_header = request.header("range")?;
        let range_spec = parse_single_range(range_header)?;
        let request_cache_control = CacheControl::from_headers(request.headers());
        let if_range = request.header("if-range").map(str::trim);
        let key = request.url().cache_key();

        if let Some(entry) = self.entries.get(&key).and_then(|entries| {
            entries
                .iter()
                .filter(|entry| entry.matches(request, auth_context))
                .max_by_key(|entry| entry.response_time)
                .cloned()
        }) {
            if !if_range_matches_entry(
                if_range,
                entry.etag.as_deref(),
                entry.last_modified.as_deref(),
            ) {
                return Some(if request_cache_control.only_if_cached {
                    RangeCacheLookup::UnsatisfiedOnlyIfCached
                } else {
                    RangeCacheLookup::IfRangeMismatch
                });
            }
            if entry.satisfies_request(&request_cache_control, now) {
                return Some(RangeCacheLookup::Hit(entry.range_response(range_spec, now)));
            }
        }

        if let Some(entry) = self.partial_entries.get(&key).and_then(|entries| {
            entries
                .iter()
                .filter(|entry| entry.matches(request, auth_context))
                .max_by_key(|entry| entry.response_time)
                .cloned()
        }) {
            if !if_range_matches_entry(
                if_range,
                Some(entry.etag.as_str()),
                entry.last_modified.as_deref(),
            ) {
                return Some(if request_cache_control.only_if_cached {
                    RangeCacheLookup::UnsatisfiedOnlyIfCached
                } else {
                    RangeCacheLookup::IfRangeMismatch
                });
            }
            let cached_range = entry
                .satisfies_request(&request_cache_control, now)
                .then(|| entry.range_response(range_spec, now))
                .flatten();
            if let Some(response) = cached_range {
                return Some(RangeCacheLookup::Hit(response));
            }
        }

        if request_cache_control.only_if_cached {
            Some(RangeCacheLookup::UnsatisfiedOnlyIfCached)
        } else {
            None
        }
    }

    fn store(&mut self, request: &Request, response: &TimedResponse, auth_context: &AuthContext) {
        if request.method() == Method::Get && response.response.status_code == 206 {
            self.store_partial(request, response, auth_context);
            return;
        }

        let Some(entry) = CacheEntry::new(request, response, auth_context.clone()) else {
            return;
        };

        let key = request.url().cache_key();
        let variants = self.entries.entry(key).or_default();
        if let Some(existing) = variants
            .iter_mut()
            .find(|existing| existing.same_variant(&entry))
        {
            if request.method() == Method::Head && !existing.response.body.is_empty() {
                if head_update_is_compatible(existing, &entry) {
                    let body = existing.response.body.clone();
                    let mut updated = entry;
                    updated.response.body = body;
                    *existing = updated;
                } else {
                    existing.freshness_lifetime = Duration::from_secs(0);
                    existing.cache_control.no_cache = true;
                }
            } else {
                *existing = entry;
            }
            return;
        }

        variants.push(entry);
    }

    fn store_partial(
        &mut self,
        request: &Request,
        timed_response: &TimedResponse,
        auth_context: &AuthContext,
    ) {
        let Some(partial) = PartialCacheEntry::new(request, timed_response, auth_context.clone())
        else {
            return;
        };

        let key = request.url().cache_key();
        let combined_entry = {
            let variants = self.partial_entries.entry(key.clone()).or_default();
            if let Some(existing) = variants
                .iter_mut()
                .find(|existing| existing.same_variant(&partial))
            {
                existing.merge_from(partial);
                existing.promote_complete()
            } else {
                let inserted = partial;
                let combined_entry = inserted.promote_complete();
                variants.push(inserted);
                combined_entry
            }
        };

        if let Some(combined) = combined_entry {
            self.upsert_complete_entry(key, combined);
        }
    }

    fn upsert_complete_entry(&mut self, key: String, entry: CacheEntry) {
        let variants = self.entries.entry(key).or_default();
        if let Some(existing) = variants
            .iter_mut()
            .find(|candidate| candidate.same_variant(&entry))
        {
            *existing = entry;
        } else {
            variants.push(entry);
        }
    }

    fn merge_not_modified(
        &mut self,
        request: &Request,
        stale: &CacheEntry,
        not_modified: &Response,
        now: SystemTime,
    ) -> Result<Response, NanoGetError> {
        let variants = self
            .entries
            .get_mut(&request.url().cache_key())
            .ok_or_else(|| NanoGetError::Cache("stale cache entry disappeared".to_string()))?;
        let existing = variants
            .iter_mut()
            .find(|entry| entry.same_variant(stale))
            .ok_or_else(|| NanoGetError::Cache("stale cache variant disappeared".to_string()))?;

        merge_headers_for_304(&mut existing.response.headers, &not_modified.headers);
        existing.cache_control = CacheControl::from_headers(&existing.response.headers);
        existing.request_time = now;
        existing.response_time = now;
        existing.freshness_lifetime = compute_freshness_lifetime(&existing.response, now);
        existing.age_header = parse_age_header(&existing.response.headers);
        existing.date_header = existing.response.header("date").and_then(parse_http_date);
        existing.etag = header_value(&existing.response.headers, "etag").map(str::to_string);
        existing.last_modified =
            header_value(&existing.response.headers, "last-modified").map(str::to_string);
        Ok(existing.response.clone())
    }
}

enum RangeCacheLookup {
    Hit(Response),
    UnsatisfiedOnlyIfCached,
    IfRangeMismatch,
}

#[derive(Clone, Copy)]
struct ByteRange {
    start: Option<usize>,
    end: Option<usize>,
}

impl ByteRange {
    fn resolve(self, total_length: usize) -> Option<(usize, usize)> {
        if total_length == 0 {
            return None;
        }

        match (self.start, self.end) {
            (Some(start), Some(end)) if start <= end && start < total_length => {
                Some((start, end.min(total_length - 1)))
            }
            (Some(start), None) if start < total_length => Some((start, total_length - 1)),
            (None, Some(suffix_len)) if suffix_len > 0 => {
                let len = suffix_len.min(total_length);
                Some((total_length - len, total_length - 1))
            }
            _ => None,
        }
    }
}

#[derive(Clone)]
struct CacheEntry {
    vary_headers: Vec<VaryHeader>,
    response: Response,
    request_time: SystemTime,
    response_time: SystemTime,
    freshness_lifetime: Duration,
    cache_control: CacheControl,
    etag: Option<String>,
    last_modified: Option<String>,
    age_header: Option<Duration>,
    date_header: Option<SystemTime>,
    auth_context: AuthContext,
}

impl CacheEntry {
    fn new(
        request: &Request,
        timed_response: &TimedResponse,
        auth_context: AuthContext,
    ) -> Option<Self> {
        let response = &timed_response.response;
        let cache_control = CacheControl::from_headers(&response.headers);
        if cache_control.no_store || !is_cacheable_status(response.status_code) {
            return None;
        }
        if auth_context.proxy.is_some() {
            return None;
        }
        if auth_context.origin.is_some() && !(cache_control.public || cache_control.private) {
            return None;
        }

        let vary_headers = extract_vary_headers(request, response)?;
        Some(Self {
            vary_headers,
            response: response.clone(),
            request_time: timed_response.request_time,
            response_time: timed_response.response_time,
            freshness_lifetime: compute_freshness_lifetime(response, timed_response.response_time),
            cache_control,
            etag: response.header("etag").map(str::to_string),
            last_modified: response.header("last-modified").map(str::to_string),
            age_header: parse_age_header(&response.headers),
            date_header: response.header("date").and_then(parse_http_date),
            auth_context,
        })
    }

    fn matches(&self, request: &Request, auth_context: &AuthContext) -> bool {
        self.auth_context == *auth_context
            && self.vary_headers.iter().all(|vary| vary.matches(request))
    }

    fn same_variant(&self, other: &Self) -> bool {
        self.vary_headers == other.vary_headers
    }

    fn is_fresh(&self, now: SystemTime) -> bool {
        if self.cache_control.no_cache {
            return false;
        }

        self.current_age(now) <= self.freshness_lifetime
    }

    fn current_age(&self, now: SystemTime) -> Duration {
        let apparent_age = match self.date_header {
            Some(date) => self
                .response_time
                .duration_since(date)
                .unwrap_or_else(|_| Duration::from_secs(0)),
            None => Duration::from_secs(0),
        };
        let corrected_received_age = self
            .age_header
            .map(|age| age.max(apparent_age))
            .unwrap_or(apparent_age);
        let response_delay = self
            .response_time
            .duration_since(self.request_time)
            .unwrap_or_else(|_| Duration::from_secs(0));
        let corrected_initial_age = corrected_received_age + response_delay;
        let resident_time = now
            .duration_since(self.response_time)
            .unwrap_or_else(|_| Duration::from_secs(0));
        corrected_initial_age + resident_time
    }

    fn remaining_freshness(&self, now: SystemTime) -> Duration {
        self.freshness_lifetime
            .saturating_sub(self.current_age(now))
    }

    fn staleness(&self, now: SystemTime) -> Duration {
        self.current_age(now)
            .saturating_sub(self.freshness_lifetime)
    }

    fn satisfies_request(&self, request_cache_control: &CacheControl, now: SystemTime) -> bool {
        if request_cache_control.no_cache || self.cache_control.no_cache {
            return false;
        }

        let age = self.current_age(now);
        if request_cache_control
            .max_age
            .is_some_and(|max_age| age > Duration::from_secs(max_age))
        {
            return false;
        }

        if request_cache_control
            .min_fresh
            .is_some_and(|min_fresh| self.remaining_freshness(now) < Duration::from_secs(min_fresh))
        {
            return false;
        }

        if self.is_fresh(now) {
            return true;
        }

        if self.cache_control.must_revalidate || self.cache_control.proxy_revalidate {
            return false;
        }

        match request_cache_control.max_stale {
            Some(None) => true,
            Some(Some(max_stale)) => self.staleness(now) <= Duration::from_secs(max_stale),
            None => false,
        }
    }

    fn response_with_age(&self, now: SystemTime) -> Response {
        let mut response = self.response.clone();
        set_age_header(&mut response.headers, self.current_age(now));
        response
    }

    fn range_response(&self, range: ByteRange, now: SystemTime) -> Response {
        match range.resolve(self.response.body.len()) {
            Some((start, end)) => {
                let mut response = self.response_with_age(now);
                response.status_code = 206;
                response.reason_phrase = "Partial Content".to_string();
                response.body = self.response.body[start..=end].to_vec();
                response
                    .headers
                    .retain(|header| !header.matches_name("content-range"));
                response
                    .headers
                    .retain(|header| !header.matches_name("content-length"));
                response.headers.push(Header::unchecked(
                    "Content-Range",
                    format!("bytes {start}-{end}/{}", self.response.body.len()),
                ));
                response.headers.push(Header::unchecked(
                    "Content-Length",
                    response.body.len().to_string(),
                ));
                response
            }
            None => {
                let mut response = self.response_with_age(now);
                response.status_code = 416;
                response.reason_phrase = "Range Not Satisfiable".to_string();
                response.body.clear();
                response
                    .headers
                    .retain(|header| !header.matches_name("content-range"));
                response
                    .headers
                    .retain(|header| !header.matches_name("content-length"));
                response.headers.push(Header::unchecked(
                    "Content-Range",
                    format!("bytes */{}", self.response.body.len()),
                ));
                response
                    .headers
                    .push(Header::unchecked("Content-Length", "0".to_string()));
                response
            }
        }
    }
}

enum CacheLookup {
    Fresh(Box<CacheEntry>),
    Stale(Box<CacheEntry>),
    UnsatisfiedOnlyIfCached,
}

#[derive(Clone)]
struct PartialCacheEntry {
    vary_headers: Vec<VaryHeader>,
    response: Response,
    request_time: SystemTime,
    response_time: SystemTime,
    freshness_lifetime: Duration,
    cache_control: CacheControl,
    age_header: Option<Duration>,
    date_header: Option<SystemTime>,
    auth_context: AuthContext,
    etag: String,
    last_modified: Option<String>,
    total_length: usize,
    segments: Vec<ByteSegment>,
}

#[derive(Clone)]
struct ByteSegment {
    start: usize,
    end: usize,
    bytes: Vec<u8>,
}

impl PartialCacheEntry {
    fn new(
        request: &Request,
        timed_response: &TimedResponse,
        auth_context: AuthContext,
    ) -> Option<Self> {
        let response = &timed_response.response;
        let cache_control = CacheControl::from_headers(&response.headers);
        if cache_control.no_store || response.status_code != 206 {
            return None;
        }
        if auth_context.proxy.is_some() {
            return None;
        }
        if auth_context.origin.is_some() && !(cache_control.public || cache_control.private) {
            return None;
        }
        let content_range = response.header("content-range")?;
        let (start, end, total_length) = parse_content_range(content_range)?;
        if end < start || end.saturating_sub(start) + 1 != response.body.len() {
            return None;
        }
        let etag = response.header("etag")?.trim().to_string();
        if !is_strong_etag(&etag) {
            return None;
        }
        let vary_headers = extract_vary_headers(request, response)?;
        Some(Self {
            vary_headers,
            response: response.clone(),
            request_time: timed_response.request_time,
            response_time: timed_response.response_time,
            freshness_lifetime: compute_freshness_lifetime(response, timed_response.response_time),
            cache_control,
            age_header: parse_age_header(&response.headers),
            date_header: response.header("date").and_then(parse_http_date),
            auth_context,
            etag,
            last_modified: response.header("last-modified").map(str::to_string),
            total_length,
            segments: vec![ByteSegment {
                start,
                end,
                bytes: response.body.clone(),
            }],
        })
    }

    fn matches(&self, request: &Request, auth_context: &AuthContext) -> bool {
        self.auth_context == *auth_context
            && self.vary_headers.iter().all(|vary| vary.matches(request))
    }

    fn same_variant(&self, other: &Self) -> bool {
        self.vary_headers == other.vary_headers
            && self.auth_context == other.auth_context
            && self.etag == other.etag
            && self.total_length == other.total_length
    }

    fn merge_from(&mut self, mut other: Self) {
        self.request_time = other.request_time;
        self.response_time = other.response_time;
        self.freshness_lifetime = other.freshness_lifetime;
        self.cache_control = other.cache_control;
        self.age_header = other.age_header;
        self.date_header = other.date_header;
        self.last_modified = other.last_modified.take();
        self.response = other.response;
        self.segments.append(&mut other.segments);
        normalize_segments(&mut self.segments);
    }

    fn current_age(&self, now: SystemTime) -> Duration {
        let apparent_age = match self.date_header {
            Some(date) => self
                .response_time
                .duration_since(date)
                .unwrap_or_else(|_| Duration::from_secs(0)),
            None => Duration::from_secs(0),
        };
        let corrected_received_age = self
            .age_header
            .map(|age| age.max(apparent_age))
            .unwrap_or(apparent_age);
        let response_delay = self
            .response_time
            .duration_since(self.request_time)
            .unwrap_or_else(|_| Duration::from_secs(0));
        let corrected_initial_age = corrected_received_age + response_delay;
        let resident_time = now
            .duration_since(self.response_time)
            .unwrap_or_else(|_| Duration::from_secs(0));
        corrected_initial_age + resident_time
    }

    fn remaining_freshness(&self, now: SystemTime) -> Duration {
        self.freshness_lifetime
            .saturating_sub(self.current_age(now))
    }

    fn staleness(&self, now: SystemTime) -> Duration {
        self.current_age(now)
            .saturating_sub(self.freshness_lifetime)
    }

    fn is_fresh(&self, now: SystemTime) -> bool {
        if self.cache_control.no_cache {
            return false;
        }
        self.current_age(now) <= self.freshness_lifetime
    }

    fn satisfies_request(&self, request_cache_control: &CacheControl, now: SystemTime) -> bool {
        if request_cache_control.no_cache || self.cache_control.no_cache {
            return false;
        }

        let age = self.current_age(now);
        if request_cache_control
            .max_age
            .is_some_and(|max_age| age > Duration::from_secs(max_age))
        {
            return false;
        }

        if request_cache_control
            .min_fresh
            .is_some_and(|min_fresh| self.remaining_freshness(now) < Duration::from_secs(min_fresh))
        {
            return false;
        }

        if self.is_fresh(now) {
            return true;
        }

        if self.cache_control.must_revalidate || self.cache_control.proxy_revalidate {
            return false;
        }

        match request_cache_control.max_stale {
            Some(None) => true,
            Some(Some(max_stale)) => self.staleness(now) <= Duration::from_secs(max_stale),
            None => false,
        }
    }

    fn range_response(&self, range: ByteRange, now: SystemTime) -> Option<Response> {
        let (start, end) = range.resolve(self.total_length)?;
        let segment = self
            .segments
            .iter()
            .find(|segment| segment.start <= start && segment.end >= end)?;
        let offset_start = start - segment.start;
        let offset_end = end - segment.start + 1;

        let mut response = self.response.clone();
        set_age_header(&mut response.headers, self.current_age(now));
        response.status_code = 206;
        response.reason_phrase = "Partial Content".to_string();
        response.body = segment.bytes[offset_start..offset_end].to_vec();
        response
            .headers
            .retain(|header| !header.matches_name("content-range"));
        response
            .headers
            .retain(|header| !header.matches_name("content-length"));
        response.headers.push(Header::unchecked(
            "Content-Range",
            format!("bytes {start}-{end}/{}", self.total_length),
        ));
        response.headers.push(Header::unchecked(
            "Content-Length",
            response.body.len().to_string(),
        ));
        Some(response)
    }

    fn promote_complete(&self) -> Option<CacheEntry> {
        if self.segments.len() != 1 {
            return None;
        }
        let segment = &self.segments[0];
        if segment.start != 0 || segment.end + 1 != self.total_length {
            return None;
        }

        let mut headers: Vec<Header> = self
            .response
            .headers
            .iter()
            .filter(|header| {
                !header.matches_name("content-range") && !header.matches_name("content-length")
            })
            .cloned()
            .collect();
        headers.push(Header::unchecked(
            "Content-Length",
            self.total_length.to_string(),
        ));

        let response = Response {
            version: self.response.version,
            status_code: 200,
            reason_phrase: "OK".to_string(),
            headers,
            trailers: Vec::new(),
            body: segment.bytes.clone(),
        };

        Some(CacheEntry {
            vary_headers: self.vary_headers.clone(),
            response,
            request_time: self.request_time,
            response_time: self.response_time,
            freshness_lifetime: self.freshness_lifetime,
            cache_control: self.cache_control,
            etag: Some(self.etag.clone()),
            last_modified: self.last_modified.clone(),
            age_header: self.age_header,
            date_header: self.date_header,
            auth_context: self.auth_context.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VaryHeader {
    name: String,
    values: Vec<String>,
}

impl VaryHeader {
    fn matches(&self, request: &Request) -> bool {
        let values: Vec<_> = request
            .headers_named(&self.name)
            .map(|header| header.value().to_string())
            .collect();
        values == self.values
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct CacheControl {
    no_store: bool,
    no_cache: bool,
    max_age: Option<u64>,
    max_stale: Option<Option<u64>>,
    min_fresh: Option<u64>,
    only_if_cached: bool,
    must_revalidate: bool,
    proxy_revalidate: bool,
    public: bool,
    private: bool,
}

impl CacheControl {
    fn from_headers(headers: &[Header]) -> Self {
        let mut directives = CacheControl::default();
        for value in headers
            .iter()
            .filter(|header| header.matches_name("cache-control"))
            .map(Header::value)
        {
            for directive in value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let (name, argument) = match directive.split_once('=') {
                    Some((name, argument)) => {
                        (name.trim().to_ascii_lowercase(), Some(argument.trim()))
                    }
                    None => (directive.to_ascii_lowercase(), None),
                };
                match name.as_str() {
                    "no-store" => directives.no_store = true,
                    "no-cache" => directives.no_cache = true,
                    "must-revalidate" => directives.must_revalidate = true,
                    "proxy-revalidate" => directives.proxy_revalidate = true,
                    "public" => directives.public = true,
                    "private" => directives.private = true,
                    "only-if-cached" => directives.only_if_cached = true,
                    "max-age" => {
                        directives.max_age = argument.and_then(parse_directive_u64);
                    }
                    "max-stale" if argument.is_none() => {
                        directives.max_stale = Some(None);
                    }
                    "max-stale" => {
                        directives.max_stale = argument.and_then(parse_directive_u64).map(Some);
                    }
                    "min-fresh" => {
                        directives.min_fresh = argument.and_then(parse_directive_u64);
                    }
                    _ => {}
                }
            }
        }

        directives
    }
}

fn response_for_method(response: &Response, method: Method) -> Response {
    if method == Method::Head {
        let mut response = response.clone();
        response.body.clear();
        response
    } else {
        response.clone()
    }
}

fn has_user_conditionals(request: &Request) -> bool {
    [
        "if-none-match",
        "if-match",
        "if-modified-since",
        "if-unmodified-since",
        "if-range",
    ]
    .iter()
    .any(|name| request.has_header(name))
}

fn extract_vary_headers(request: &Request, response: &Response) -> Option<Vec<VaryHeader>> {
    let vary_names: Vec<String> = response
        .headers_named("vary")
        .flat_map(|header| header.value().split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    if vary_names.is_empty() {
        return Some(Vec::new());
    }
    if vary_names.iter().any(|value| value == "*") {
        return None;
    }

    Some(
        vary_names
            .iter()
            .map(|name| VaryHeader {
                name: name.clone(),
                values: request
                    .headers_named(name)
                    .map(|header| header.value().to_string())
                    .collect(),
            })
            .collect(),
    )
}

fn is_cacheable_status(status_code: u16) -> bool {
    matches!(
        status_code,
        200 | 203 | 204 | 300 | 301 | 404 | 405 | 410 | 414 | 501
    )
}

fn compute_freshness_lifetime(response: &Response, now: SystemTime) -> Duration {
    let cache_control = CacheControl::from_headers(&response.headers);
    if let Some(max_age) = cache_control.max_age {
        return Duration::from_secs(max_age);
    }

    if let Some(expires) = response.header("expires").and_then(parse_http_date) {
        if let Some(date) = response.header("date").and_then(parse_http_date) {
            return expires
                .duration_since(date)
                .unwrap_or_else(|_| Duration::from_secs(0));
        }
        return expires
            .duration_since(now)
            .unwrap_or_else(|_| Duration::from_secs(0));
    }

    if let (Some(last_modified), Some(date)) = (
        response.header("last-modified").and_then(parse_http_date),
        response.header("date").and_then(parse_http_date),
    ) {
        return date
            .duration_since(last_modified)
            .map(|age| (age / 10).min(Duration::from_secs(86_400)))
            .unwrap_or(Duration::from_secs(0));
    }

    Duration::from_secs(0)
}

fn header_value<'a>(headers: &'a [Header], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|header| header.matches_name(name))
        .map(Header::value)
}

fn merge_headers_for_304(stored: &mut Vec<Header>, fresh: &[Header]) {
    for header in fresh {
        if header.matches_name("content-length")
            || header.matches_name("transfer-encoding")
            || header.matches_name("content-range")
        {
            continue;
        }
        stored.retain(|existing| !existing.matches_name(header.name()));
        stored.push(header.clone());
    }
}

fn parse_age_header(headers: &[Header]) -> Option<Duration> {
    header_value(headers, "age")
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

fn set_age_header(headers: &mut Vec<Header>, age: Duration) {
    let age_seconds = age.as_secs().to_string();
    headers.retain(|header| !header.matches_name("age"));
    headers.push(Header::unchecked("Age", age_seconds));
}

fn head_update_is_compatible(existing: &CacheEntry, candidate: &CacheEntry) -> bool {
    let current_etag = existing.etag.as_deref();
    let candidate_etag = candidate.etag.as_deref();
    if let (Some(current), Some(next)) = (current_etag, candidate_etag) {
        if current != next {
            return false;
        }
    }

    if candidate
        .response
        .header("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|content_length| content_length != existing.response.body.len())
    {
        return false;
    }

    true
}

fn parse_directive_u64(value: &str) -> Option<u64> {
    let value = value.trim().trim_matches('"');
    value.parse::<u64>().ok()
}

fn parse_single_range(value: &str) -> Option<ByteRange> {
    let value = value.trim();
    let bytes = value.strip_prefix("bytes=")?.trim();
    if bytes.contains(',') {
        return None;
    }
    let (start, end) = bytes.split_once('-')?;
    if start.is_empty() {
        return Some(ByteRange {
            start: None,
            end: end.parse().ok(),
        });
    }
    let start_value = start.parse().ok()?;
    if end.is_empty() {
        return Some(ByteRange {
            start: Some(start_value),
            end: None,
        });
    }
    Some(ByteRange {
        start: Some(start_value),
        end: end.parse().ok(),
    })
}

fn parse_content_range(value: &str) -> Option<(usize, usize, usize)> {
    let value = value.trim();
    let bytes = value.strip_prefix("bytes ")?;
    let (range, complete_length) = bytes.split_once('/')?;
    let complete_length = complete_length.parse().ok()?;
    let (start, end) = range.split_once('-')?;
    let start = start.parse().ok()?;
    let end = end.parse().ok()?;
    Some((start, end, complete_length))
}

fn is_strong_etag(value: &str) -> bool {
    let value = value.trim();
    value.starts_with('"')
        && value.ends_with('"')
        && !value.starts_with("W/")
        && !value.starts_with("w/")
}

fn if_range_matches_entry(
    if_range: Option<&str>,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> bool {
    let Some(if_range) = if_range else {
        return true;
    };

    if if_range.starts_with('"') || if_range.starts_with("W/") || if_range.starts_with("w/") {
        return etag.is_some_and(|cached| cached == if_range);
    }

    let Some(if_range_date) = parse_http_date(if_range) else {
        return false;
    };
    let Some(last_modified_date) = last_modified.and_then(parse_http_date) else {
        return false;
    };
    last_modified_date <= if_range_date
}

fn validate_request_conditionals(request: &Request) -> Result<(), NanoGetError> {
    let Some(if_range) = request.header("if-range") else {
        return Ok(());
    };
    if !request.has_header("range") {
        return Err(NanoGetError::InvalidConditionalRequest(
            "If-Range requires Range".to_string(),
        ));
    }
    let if_range = if_range.trim();
    if if_range.starts_with("W/") || if_range.starts_with("w/") {
        return Err(NanoGetError::InvalidConditionalRequest(
            "weak ETags are not valid in If-Range".to_string(),
        ));
    }
    Ok(())
}

fn normalize_segments(segments: &mut Vec<ByteSegment>) {
    if segments.is_empty() {
        return;
    }
    segments.sort_by_key(|segment| segment.start);
    let mut merged: Vec<ByteSegment> = Vec::with_capacity(segments.len());

    for segment in segments.drain(..) {
        if let Some(last) = merged.last_mut() {
            if segment.start <= last.end.saturating_add(1) {
                let merged_start = last.start;
                let merged_end = last.end.max(segment.end);
                let mut bytes = vec![0u8; merged_end - merged_start + 1];
                let last_offset = last.start - merged_start;
                bytes[last_offset..last_offset + last.bytes.len()].copy_from_slice(&last.bytes);
                let segment_offset = segment.start - merged_start;
                bytes[segment_offset..segment_offset + segment.bytes.len()]
                    .copy_from_slice(&segment.bytes);
                last.start = merged_start;
                last.end = merged_end;
                last.bytes = bytes;
                continue;
            }
        }
        merged.push(segment);
    }

    *segments = merged;
}

fn validate_proxy_header_name(name: &str) -> Result<(), NanoGetError> {
    match name.to_ascii_lowercase().as_str() {
        "host" | "connection" | "content-length" | "transfer-encoding" | "trailer" | "upgrade" => {
            Err(NanoGetError::ProtocolManagedHeader(name.to_string()))
        }
        "keep-alive" | "proxy-connection" | "te" => {
            Err(NanoGetError::HopByHopHeader(name.to_string()))
        }
        _ => Ok(()),
    }
}

fn effective_auth_context(request: &Request, config: &ClientConfig) -> AuthContext {
    AuthContext {
        origin: request
            .header("authorization")
            .map(str::to_string)
            .or_else(|| config.preemptive_authorization.clone()),
        proxy: request
            .header("proxy-authorization")
            .map(str::to_string)
            .or_else(|| config.preemptive_proxy_authorization.clone())
            .filter(|_| config.proxy.is_some()),
    }
}

fn prepared_connect_headers(
    request: &Request,
    config: &ClientConfig,
    proxy: &ProxyConfig,
) -> Result<Vec<Header>, NanoGetError> {
    let mut headers: Vec<Header> = proxy.headers().to_vec();
    headers.retain(|header| !header.matches_name("authorization"));

    if headers
        .iter()
        .any(|header| header.matches_name("proxy-authorization"))
    {
        return Ok(headers);
    }

    let proxy_authorization = request
        .header("proxy-authorization")
        .map(str::to_string)
        .or_else(|| config.preemptive_proxy_authorization.clone());
    if let Some(value) = proxy_authorization {
        headers.push(Header::new("Proxy-Authorization", value)?);
    }

    Ok(headers)
}

fn maybe_retry_request_auth(
    handler: Option<&DynAuthHandler>,
    target: AuthTarget,
    request: &Request,
    response: &Response,
    seen_challenges: &mut Vec<Vec<Challenge>>,
) -> Result<Option<Request>, NanoGetError> {
    let header_name = match target {
        AuthTarget::Origin => "www-authenticate",
        AuthTarget::Proxy => "proxy-authenticate",
    };
    if !response
        .headers
        .iter()
        .any(|header| header.matches_name(header_name))
    {
        return Ok(None);
    }

    let existing_header = match target {
        AuthTarget::Origin => "authorization",
        AuthTarget::Proxy => "proxy-authorization",
    };
    if request.has_header(existing_header) {
        return Ok(None);
    }

    let challenges = parse_authenticate_headers(&response.headers, header_name)?;
    if challenges.is_empty() {
        return Ok(None);
    }
    if seen_challenges
        .iter()
        .any(|previous| previous == &challenges)
    {
        return Ok(None);
    }
    seen_challenges.push(challenges.clone());

    let Some(handler) = handler else {
        return Ok(None);
    };

    match handler.respond(target, request.url(), &challenges, request, response)? {
        AuthDecision::UseHeaders(headers) => {
            let mut retry = request.clone();
            retry.remove_headers_named(existing_header);
            for header in headers {
                if header.matches_name(existing_header) {
                    retry.set_header(header.name().to_string(), header.value().to_string())?;
                }
            }
            Ok(Some(retry))
        }
        AuthDecision::NoMatch => Ok(None),
        AuthDecision::Abort => Err(NanoGetError::AuthenticationRejected(match target {
            AuthTarget::Origin => "origin authentication handler aborted".to_string(),
            AuthTarget::Proxy => "proxy authentication handler aborted".to_string(),
        })),
    }
}

fn gateway_timeout_response() -> Response {
    Response {
        version: crate::response::HttpVersion::Http11,
        status_code: 504,
        reason_phrase: "Gateway Timeout".to_string(),
        headers: Vec::new(),
        trailers: Vec::new(),
        body: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::SystemTime;

    use std::time::{Duration, UNIX_EPOCH};

    use super::{
        if_range_matches_entry, is_strong_etag, normalize_segments, parse_content_range,
        parse_single_range, prepared_connect_headers, validate_request_conditionals, AuthContext,
        ByteRange, ByteSegment, CacheControl, CacheEntry, CacheMode, Client, ClientBuilder,
        ClientConfig, ConnectionPolicy, MemoryCache, ParserStrictness, PartialCacheEntry,
        ProxyConfig, TimedResponse,
    };
    use crate::auth::{AuthDecision, AuthHandler, AuthTarget, Challenge};
    use crate::request::{Header, Method, RedirectPolicy, Request};
    use crate::response::{HttpVersion, Response};
    use crate::url::Url;

    struct AbortAuthHandler;

    impl AuthHandler for AbortAuthHandler {
        fn respond(
            &self,
            _target: AuthTarget,
            _url: &Url,
            _challenges: &[Challenge],
            _request: &Request,
            _response: &Response,
        ) -> Result<AuthDecision, crate::NanoGetError> {
            Ok(AuthDecision::Abort)
        }
    }

    fn timed_response(response: Response) -> TimedResponse {
        let now = SystemTime::now();
        TimedResponse {
            response,
            request_time: now,
            response_time: now,
        }
    }

    fn response(status: u16, headers: Vec<crate::Header>, body: &[u8]) -> Response {
        Response {
            version: HttpVersion::Http11,
            status_code: status,
            reason_phrase: "OK".to_string(),
            headers,
            trailers: Vec::new(),
            body: body.to_vec(),
        }
    }

    struct NoMatchAuthHandler;

    impl AuthHandler for NoMatchAuthHandler {
        fn respond(
            &self,
            _target: AuthTarget,
            _url: &Url,
            _challenges: &[Challenge],
            _request: &Request,
            _response: &Response,
        ) -> Result<AuthDecision, crate::NanoGetError> {
            Ok(AuthDecision::NoMatch)
        }
    }

    struct UseHeadersAuthHandler {
        header_name: &'static str,
        header_value: &'static str,
    }

    impl AuthHandler for UseHeadersAuthHandler {
        fn respond(
            &self,
            _target: AuthTarget,
            _url: &Url,
            _challenges: &[Challenge],
            _request: &Request,
            _response: &Response,
        ) -> Result<AuthDecision, crate::NanoGetError> {
            Ok(AuthDecision::UseHeaders(vec![
                Header::unchecked("X-Ignored", "1"),
                Header::unchecked(self.header_name, self.header_value),
            ]))
        }
    }

    fn spawn_recording_server(
        responses: Vec<Vec<u8>>,
    ) -> (u16, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::<String>::new()));
        let requests_for_thread = Arc::clone(&requests);
        let handle = thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut chunk = [0u8; 256];
                let mut read = stream.read(&mut chunk).unwrap();
                while read > 0 {
                    request.extend_from_slice(&chunk[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                    read = stream.read(&mut chunk).unwrap();
                }
                requests_for_thread
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&request).into_owned());
                stream.write_all(&response).unwrap();
            }
        });
        (port, requests, handle)
    }

    #[test]
    fn builder_configures_client() {
        let proxy = ProxyConfig::new("http://127.0.0.1:8080").unwrap();
        let client = Client::builder()
            .redirect_policy(RedirectPolicy::follow(4))
            .connection_policy(ConnectionPolicy::Reuse)
            .cache_mode(CacheMode::Memory)
            .proxy(proxy)
            .build();

        let session = client.session();
        let request = Request::get("http://example.com").unwrap();
        assert_eq!(session.config.redirect_policy, RedirectPolicy::follow(4));
        assert_eq!(session.config.parser_strictness, ParserStrictness::Strict);
        assert_eq!(request.method(), Method::Get);
    }

    #[test]
    fn cache_control_parser_recognizes_directives() {
        let mut request = Request::get("http://example.com").unwrap();
        request
            .add_header(
                "Cache-Control",
                "no-store, max-age=30, no-cache, max-stale=10, min-fresh=4, only-if-cached",
            )
            .unwrap();
        let cache_control = CacheControl::from_headers(request.headers());
        assert!(cache_control.no_store);
        assert!(cache_control.no_cache);
        assert_eq!(cache_control.max_age, Some(30));
        assert_eq!(cache_control.max_stale, Some(Some(10)));
        assert_eq!(cache_control.min_fresh, Some(4));
        assert!(cache_control.only_if_cached);
    }

    #[test]
    fn default_client_builder_matches_release_defaults() {
        let client = ClientBuilder::default().build();
        let session = client.session();
        assert_eq!(session.config.connection_policy, ConnectionPolicy::Close);
        assert_eq!(session.config.cache_mode, CacheMode::Disabled);
        assert_eq!(session.config.redirect_policy, RedirectPolicy::none());
    }

    #[test]
    fn request_date_helpers_accept_cacheable_times() {
        let mut request = Request::get("http://example.com").unwrap();
        request
            .if_modified_since(UNIX_EPOCH + Duration::from_secs(784_111_777))
            .unwrap();
        assert!(request.header("if-modified-since").is_some());
    }

    #[test]
    fn cache_control_parser_recognizes_bare_max_stale() {
        let mut request = Request::get("http://example.com").unwrap();
        request.add_header("Cache-Control", "max-stale").unwrap();
        let cache_control = CacheControl::from_headers(request.headers());
        assert_eq!(cache_control.max_stale, Some(None));
    }

    #[test]
    fn cache_control_parser_recognizes_revalidation_and_visibility_directives() {
        let headers = vec![crate::request::Header::unchecked(
            "Cache-Control",
            "must-revalidate, proxy-revalidate, public, private",
        )];
        let cache_control = CacheControl::from_headers(&headers);
        assert!(cache_control.must_revalidate);
        assert!(cache_control.proxy_revalidate);
        assert!(cache_control.public);
        assert!(cache_control.private);
    }

    #[test]
    fn cache_control_parser_is_case_insensitive_and_handles_quoted_values() {
        let headers = vec![crate::request::Header::unchecked(
            "Cache-Control",
            "MAX-AGE=\"60\", MIN-FRESH=5, ONLY-IF-CACHED",
        )];
        let cache_control = CacheControl::from_headers(&headers);
        assert_eq!(cache_control.max_age, Some(60));
        assert_eq!(cache_control.min_fresh, Some(5));
        assert!(cache_control.only_if_cached);
    }

    #[test]
    fn builder_sets_parser_strictness() {
        let client = Client::builder()
            .parser_strictness(ParserStrictness::Lenient)
            .build();
        let session = client.session();
        assert_eq!(session.config.parser_strictness, ParserStrictness::Lenient);
    }

    #[test]
    fn proxy_config_validates_scheme_and_headers() {
        let error = ProxyConfig::new("https://example.com").unwrap_err();
        assert!(matches!(
            error,
            crate::NanoGetError::UnsupportedProxyScheme(_)
        ));

        let mut proxy = ProxyConfig::new("http://example.com:8080").unwrap();
        proxy.add_header("X-Proxy", "yes").unwrap();
        assert_eq!(proxy.url().authority_form(), "example.com:8080");
        assert_eq!(proxy.headers().len(), 1);
    }

    #[test]
    fn validate_request_conditionals_checks_if_range_requirements() {
        let request = Request::get("http://example.com").unwrap();
        assert!(validate_request_conditionals(&request).is_ok());

        let mut invalid = Request::get("http://example.com").unwrap();
        invalid.if_range("\"v1\"").unwrap();
        assert!(matches!(
            validate_request_conditionals(&invalid),
            Err(crate::NanoGetError::InvalidConditionalRequest(_))
        ));

        let mut weak = Request::get("http://example.com").unwrap();
        weak.range_bytes(Some(0), Some(1)).unwrap();
        weak.if_range("W/\"v1\"").unwrap();
        assert!(matches!(
            validate_request_conditionals(&weak),
            Err(crate::NanoGetError::InvalidConditionalRequest(_))
        ));
    }

    #[test]
    fn parse_and_range_helpers_cover_edge_cases() {
        assert!(parse_single_range("bytes=0-2").is_some());
        assert!(parse_single_range("bytes=2-").is_some());
        assert!(parse_single_range("bytes=-4").is_some());
        assert!(parse_single_range("bytes=0-1,3-4").is_none());
        assert!(parse_content_range("bytes 0-1/4").is_some());
        assert!(parse_content_range("invalid").is_none());
        assert!(is_strong_etag("\"v1\""));
        assert!(!is_strong_etag("W/\"v1\""));

        let suffix = ByteRange {
            start: None,
            end: Some(2),
        };
        assert_eq!(suffix.resolve(5), Some((3, 4)));
        let invalid = ByteRange {
            start: Some(10),
            end: Some(12),
        };
        assert_eq!(invalid.resolve(5), None);
        let open_ended = ByteRange {
            start: Some(2),
            end: None,
        };
        assert_eq!(open_ended.resolve(5), Some((2, 4)));
        let empty = ByteRange {
            start: Some(0),
            end: Some(0),
        };
        assert_eq!(empty.resolve(0), None);
    }

    #[test]
    fn if_range_matching_handles_etags_and_dates() {
        assert!(if_range_matches_entry(None, None, None));
        assert!(if_range_matches_entry(Some("\"v1\""), Some("\"v1\""), None));
        assert!(!if_range_matches_entry(
            Some("\"v1\""),
            Some("\"v2\""),
            None
        ));
        assert!(if_range_matches_entry(
            Some("Sun, 06 Nov 1994 08:49:37 GMT"),
            None,
            Some("Sun, 06 Nov 1994 08:49:37 GMT")
        ));
        assert!(!if_range_matches_entry(
            Some("Sun, 06 Nov 1994 08:49:37 GMT"),
            None,
            Some("Sun, 07 Nov 1994 08:49:37 GMT")
        ));
    }

    #[test]
    fn normalize_segments_merges_adjacent_and_overlapping_ranges() {
        let mut segments = Vec::new();
        normalize_segments(&mut segments);
        assert!(segments.is_empty());

        let mut segments = vec![
            ByteSegment {
                start: 0,
                end: 2,
                bytes: b"abc".to_vec(),
            },
            ByteSegment {
                start: 2,
                end: 4,
                bytes: b"cde".to_vec(),
            },
            ByteSegment {
                start: 8,
                end: 9,
                bytes: b"xy".to_vec(),
            },
        ];
        normalize_segments(&mut segments);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].bytes, b"abcde");
    }

    #[test]
    fn cache_entry_range_response_handles_satisfiable_and_unsatisfiable_ranges() {
        let request = Request::get("http://example.com").unwrap();
        let cached = response(
            200,
            vec![crate::Header::unchecked("Content-Length", "6")],
            b"abcdef",
        );
        let entry =
            CacheEntry::new(&request, &timed_response(cached), AuthContext::default()).unwrap();
        let partial = entry.range_response(
            ByteRange {
                start: Some(1),
                end: Some(3),
            },
            SystemTime::now(),
        );
        assert_eq!(partial.status_code, 206);
        assert_eq!(partial.body, b"bcd");

        let unsatisfiable = entry.range_response(
            ByteRange {
                start: Some(9),
                end: Some(12),
            },
            SystemTime::now(),
        );
        assert_eq!(unsatisfiable.status_code, 416);
    }

    #[test]
    fn partial_cache_entry_promotes_complete_when_segments_cover_the_representation() {
        let request = Request::get("http://example.com").unwrap();
        let first = response(
            206,
            vec![
                crate::Header::unchecked("Cache-Control", "max-age=60"),
                crate::Header::unchecked("ETag", "\"v1\""),
                crate::Header::unchecked("Content-Range", "bytes 0-2/6"),
                crate::Header::unchecked("Content-Length", "3"),
            ],
            b"abc",
        );
        let second = response(
            206,
            vec![
                crate::Header::unchecked("Cache-Control", "max-age=60"),
                crate::Header::unchecked("ETag", "\"v1\""),
                crate::Header::unchecked("Content-Range", "bytes 3-5/6"),
                crate::Header::unchecked("Content-Length", "3"),
            ],
            b"def",
        );
        let mut partial =
            PartialCacheEntry::new(&request, &timed_response(first), AuthContext::default())
                .unwrap();
        partial.merge_from(
            PartialCacheEntry::new(&request, &timed_response(second), AuthContext::default())
                .unwrap(),
        );
        let promoted = partial.promote_complete().unwrap();
        assert_eq!(promoted.response.status_code, 200);
        assert_eq!(promoted.response.body, b"abcdef");
    }

    #[test]
    fn memory_cache_stores_206_segments_and_promotes_to_full_entry() {
        let mut cache = MemoryCache::default();
        let url = "http://example.com/path";

        let mut first = Request::get(url).unwrap();
        first.range_bytes(Some(0), Some(2)).unwrap();
        let response_one = response(
            206,
            vec![
                crate::Header::unchecked("Cache-Control", "max-age=60"),
                crate::Header::unchecked("ETag", "\"v1\""),
                crate::Header::unchecked("Content-Range", "bytes 0-2/6"),
                crate::Header::unchecked("Content-Length", "3"),
            ],
            b"abc",
        );
        cache.store(
            &first,
            &timed_response(response_one),
            &AuthContext::default(),
        );

        let mut second = Request::get(url).unwrap();
        second.range_bytes(Some(3), Some(5)).unwrap();
        let response_two = response(
            206,
            vec![
                crate::Header::unchecked("Cache-Control", "max-age=60"),
                crate::Header::unchecked("ETag", "\"v1\""),
                crate::Header::unchecked("Content-Range", "bytes 3-5/6"),
                crate::Header::unchecked("Content-Length", "3"),
            ],
            b"def",
        );
        cache.store(
            &second,
            &timed_response(response_two),
            &AuthContext::default(),
        );

        let full = Request::get(url).unwrap();
        let lookup = cache.lookup(&full, SystemTime::now(), &AuthContext::default());
        assert!(matches!(
            lookup,
            Some(super::CacheLookup::Fresh(ref entry)) if entry.response.body == b"abcdef"
        ));
    }

    #[test]
    fn prepared_connect_headers_respects_request_and_preemptive_values() {
        let mut proxy = ProxyConfig::new("http://127.0.0.1:8080").unwrap();
        proxy.add_header("Authorization", "drop-me").unwrap();
        let mut request = Request::get("https://example.com").unwrap();
        request.proxy_authorization("Basic cmVxOnByb3h5").unwrap();

        let config = ClientConfig {
            preemptive_proxy_authorization: Some("Basic cHJlZW1wdGl2ZQ==".to_string()),
            ..ClientConfig::default()
        };

        let headers = prepared_connect_headers(&request, &config, &proxy).unwrap();
        assert!(headers
            .iter()
            .any(|header| header.matches_name("proxy-authorization")));
        assert!(!headers
            .iter()
            .any(|header| header.matches_name("authorization")));

        let mut proxy_with_auth = ProxyConfig::new("http://127.0.0.1:8080").unwrap();
        proxy_with_auth
            .add_header("Proxy-Authorization", "Basic cHJveHk=")
            .unwrap();
        let headers = prepared_connect_headers(&request, &config, &proxy_with_auth).unwrap();
        assert_eq!(
            headers
                .iter()
                .filter(|header| header.matches_name("proxy-authorization"))
                .count(),
            1
        );
        assert!(headers
            .iter()
            .any(|header| header.value() == "Basic cHJveHk="));
    }

    #[test]
    fn maybe_retry_auth_handler_abort_maps_to_authentication_rejected() {
        let handler: Arc<dyn AuthHandler + Send + Sync> = Arc::new(AbortAuthHandler);
        let request = Request::get("http://example.com").unwrap();
        let response = response(
            401,
            vec![crate::Header::unchecked(
                "WWW-Authenticate",
                "Basic realm=\"api\"",
            )],
            b"",
        );
        let mut seen = Vec::new();
        let error = super::maybe_retry_request_auth(
            Some(&handler),
            AuthTarget::Origin,
            &request,
            &response,
            &mut seen,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            crate::NanoGetError::AuthenticationRejected(_)
        ));
    }

    #[test]
    fn execute_pipelined_validates_requests_before_network_io() {
        let mut close_session = Client::builder().build().session();
        let empty = close_session.execute_pipelined(&[]).unwrap();
        assert!(empty.is_empty());

        let request = Request::get("http://example.com").unwrap();
        let error = close_session.execute_pipelined(&[request]).unwrap_err();
        assert!(matches!(error, crate::NanoGetError::Pipeline(_)));

        let mut invalid_conditional = Request::get("http://example.com").unwrap();
        invalid_conditional.if_range("\"v1\"").unwrap();
        let mut reuse_session = Client::builder()
            .connection_policy(ConnectionPolicy::Reuse)
            .build()
            .session();
        let error = reuse_session
            .execute_pipelined(&[invalid_conditional])
            .unwrap_err();
        assert!(matches!(
            error,
            crate::NanoGetError::InvalidConditionalRequest(_)
        ));

        let mut reuse_session = Client::builder()
            .connection_policy(ConnectionPolicy::Reuse)
            .build()
            .session();
        let error = reuse_session
            .execute_pipelined(&[
                Request::get("http://example.com/one").unwrap(),
                Request::get("http://example.org/two").unwrap(),
            ])
            .unwrap_err();
        assert!(matches!(error, crate::NanoGetError::Pipeline(_)));

        let mut reuse_session = Client::builder()
            .connection_policy(ConnectionPolicy::Reuse)
            .build()
            .session();
        let error = reuse_session
            .execute_pipelined(&[
                Request::get("https://example.com/one").unwrap(),
                Request::get("https://example.org/two").unwrap(),
            ])
            .unwrap_err();
        assert!(matches!(error, crate::NanoGetError::Pipeline(_)));
    }

    #[test]
    fn execute_follow_redirects_without_location_returns_original_response() {
        let (port, _requests, handle) = spawn_recording_server(vec![
            b"HTTP/1.1 302 Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
        ]);
        let mut session = Client::builder().build().session();
        let request = Request::get(format!("http://127.0.0.1:{port}/"))
            .unwrap()
            .with_redirect_policy(RedirectPolicy::follow(3));
        let response = session.execute(request).unwrap();
        assert_eq!(response.status_code, 302);
        handle.join().unwrap();
    }

    #[test]
    fn session_execute_ref_clones_and_executes_requests() {
        let (port, _requests, handle) = spawn_recording_server(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec(),
        ]);
        let mut session = Client::builder().build().session();
        let request = Request::get(format!("http://127.0.0.1:{port}/")).unwrap();
        let response = session.execute_ref(&request).unwrap();
        assert_eq!(response.body, b"ok");
        handle.join().unwrap();
    }

    #[test]
    fn execute_pipelined_handles_connection_close_on_final_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 256];
            loop {
                let read = stream.read(&mut chunk).unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..read]);
                if request
                    .windows(8)
                    .filter(|window| *window == b"HTTP/1.1")
                    .count()
                    >= 2
                {
                    break;
                }
            }
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\naHTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 1\r\n\r\nb",
                )
                .unwrap();
        });

        let mut session = Client::builder()
            .connection_policy(ConnectionPolicy::Reuse)
            .build()
            .session();
        let responses = session
            .execute_pipelined(&[
                Request::get(format!("http://127.0.0.1:{port}/one")).unwrap(),
                Request::get(format!("http://127.0.0.1:{port}/two")).unwrap(),
            ])
            .unwrap();
        assert_eq!(responses.len(), 2);
        assert_eq!(responses[0].body, b"a");
        assert_eq!(responses[1].body, b"b");
        assert!(session.connection.is_none());
        handle.join().unwrap();
    }

    #[test]
    fn execute_one_only_if_cached_without_memory_cache_returns_gateway_timeout() {
        let mut session = Client::builder().build().session();
        let mut request = Request::get("http://example.com").unwrap();
        request
            .add_header("Cache-Control", "only-if-cached")
            .unwrap();
        let response = session.execute_one(request).unwrap();
        assert_eq!(response.status_code, 504);
    }

    #[test]
    fn execute_one_uses_range_cache_hits_and_if_range_mismatch_fallback() {
        let (port, requests, handle) = spawn_recording_server(vec![
            b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 6\r\n\r\nghijkl"
                .to_vec(),
        ]);
        let url = format!("http://127.0.0.1:{port}/range");
        let request = Request::get(&url).unwrap();
        let cached = response(
            200,
            vec![
                Header::unchecked("Cache-Control", "max-age=60"),
                Header::unchecked("ETag", "\"v1\""),
                Header::unchecked("Content-Length", "6"),
            ],
            b"abcdef",
        );
        let mut session = Client::builder()
            .cache_mode(CacheMode::Memory)
            .build()
            .session();
        session.cache.lock().unwrap().store(
            &request,
            &timed_response(cached),
            &AuthContext::default(),
        );

        let mut range_hit = Request::get(&url).unwrap();
        range_hit.range_bytes(Some(1), Some(3)).unwrap();
        let hit = session.execute_one(range_hit).unwrap();
        assert_eq!(hit.status_code, 206);
        assert_eq!(hit.body, b"bcd");

        let mut mismatch = Request::get(&url).unwrap();
        mismatch.range_bytes(Some(0), Some(2)).unwrap();
        mismatch.if_range("\"different\"").unwrap();
        let network = session.execute_one(mismatch).unwrap();
        assert_eq!(network.status_code, 200);
        assert_eq!(network.body, b"ghijkl");
        assert_eq!(requests.lock().unwrap().len(), 1);
        handle.join().unwrap();
    }

    #[test]
    fn execute_stale_adds_if_modified_since_when_last_modified_exists() {
        let (port, requests, handle) = spawn_recording_server(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec(),
        ]);
        let request = Request::get(format!("http://127.0.0.1:{port}/stale")).unwrap();
        let stale_response = response(
            200,
            vec![
                Header::unchecked("Cache-Control", "max-age=0"),
                Header::unchecked("Last-Modified", "Sun, 06 Nov 1994 08:49:37 GMT"),
                Header::unchecked("Date", "Sun, 06 Nov 1994 08:49:37 GMT"),
                Header::unchecked("Content-Length", "4"),
            ],
            b"body",
        );
        let stale = CacheEntry::new(
            &request,
            &timed_response(stale_response),
            AuthContext::default(),
        )
        .unwrap();
        let mut session = Client::builder()
            .cache_mode(CacheMode::Memory)
            .build()
            .session();
        let response = session.execute_stale(request.clone(), stale).unwrap();
        assert_eq!(response.status_code, 200);
        let captured = requests.lock().unwrap().join("\n");
        assert!(captured.to_ascii_lowercase().contains("if-modified-since:"));
        handle.join().unwrap();
    }

    #[test]
    fn execute_stale_keeps_existing_user_conditionals() {
        let (port, requests, handle) = spawn_recording_server(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec(),
        ]);
        let mut request = Request::get(format!("http://127.0.0.1:{port}/stale")).unwrap();
        request.if_none_match("\"client\"").unwrap();
        let stale_response = response(
            200,
            vec![
                Header::unchecked("Cache-Control", "max-age=0"),
                Header::unchecked("ETag", "\"server\""),
                Header::unchecked("Last-Modified", "Sun, 06 Nov 1994 08:49:37 GMT"),
                Header::unchecked("Date", "Sun, 06 Nov 1994 08:49:37 GMT"),
                Header::unchecked("Content-Length", "4"),
            ],
            b"body",
        );
        let stale = CacheEntry::new(
            &request,
            &timed_response(stale_response),
            AuthContext::default(),
        )
        .unwrap();
        let mut session = Client::builder()
            .cache_mode(CacheMode::Memory)
            .build()
            .session();
        let response = session.execute_stale(request, stale).unwrap();
        assert_eq!(response.status_code, 200);
        let captured = requests.lock().unwrap().join("\n").to_ascii_lowercase();
        assert!(captured.contains("if-none-match: \"client\""));
        handle.join().unwrap();
    }

    #[test]
    fn send_request_retries_network_errors_but_not_protocol_errors() {
        let mut session = Client::builder()
            .connection_policy(ConnectionPolicy::Reuse)
            .build()
            .session();
        let request = Request::get("http://127.0.0.1:9").unwrap();
        let error = session.send_request(&request).unwrap_err();
        assert!(matches!(
            error,
            crate::NanoGetError::Connect(_) | crate::NanoGetError::Io(_)
        ));

        let (port, _requests, handle) = spawn_recording_server(vec![b"BROKEN\r\n\r\n".to_vec()]);
        let mut session = Client::builder()
            .connection_policy(ConnectionPolicy::Reuse)
            .build()
            .session();
        let request = Request::get(format!("http://127.0.0.1:{port}/")).unwrap();
        let error = session.send_request(&request).unwrap_err();
        assert!(matches!(error, crate::NanoGetError::MalformedStatusLine(_)));
        handle.join().unwrap();
    }

    struct AlwaysIoStream;

    impl Read for AlwaysIoStream {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "forced read",
            ))
        }
    }

    impl Write for AlwaysIoStream {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "forced write",
            ))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn send_request_retry_branch_runs_for_live_connection_io_errors() {
        let mut stream = AlwaysIoStream;
        let mut buf = [0u8; 1];
        assert!(stream.read(&mut buf).is_err());
        assert!(stream.write(&buf).is_err());
        stream.flush().unwrap();

        let mut session = Client::builder()
            .connection_policy(ConnectionPolicy::Reuse)
            .build()
            .session();
        let request = Request::get("http://127.0.0.1:9").unwrap();
        session.connection = Some(super::LiveConnection {
            key: super::connection_key(&session.config.proxy, request.url()),
            reader: std::io::BufReader::new(Box::new(AlwaysIoStream)),
        });
        let error = session.send_request(&request).unwrap_err();
        assert!(matches!(
            error,
            crate::NanoGetError::Connect(_) | crate::NanoGetError::Io(_)
        ));
    }

    #[test]
    fn execute_pipelined_propagates_response_parse_errors() {
        let (port, _requests, handle) = spawn_recording_server(vec![b"NOT-HTTP\r\n\r\n".to_vec()]);
        let mut session = Client::builder()
            .connection_policy(ConnectionPolicy::Reuse)
            .build()
            .session();
        let request = Request::get(format!("http://127.0.0.1:{port}/bad")).unwrap();
        let error = session.execute_pipelined(&[request]).unwrap_err();
        assert!(matches!(error, crate::NanoGetError::MalformedStatusLine(_)));
        handle.join().unwrap();
    }

    #[test]
    fn send_ephemeral_propagates_prepared_request_failures() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let _ = listener.accept().unwrap();
        });

        let proxy = super::ProxyConfig {
            url: Url::parse(format!("http://127.0.0.1:{port}").as_str()).unwrap(),
            headers: vec![Header::unchecked("X-Bad", "line\nbreak")],
        };
        let session = Client::builder().proxy(proxy).build().session();
        let request = Request::get("http://example.com").unwrap();
        let error = session.send_ephemeral(&request).unwrap_err();
        assert!(matches!(error, crate::NanoGetError::InvalidHeaderValue(_)));
        handle.join().unwrap();
    }

    #[test]
    fn send_helpers_cover_ephemeral_and_live_connection_paths() {
        let (port, _requests, handle) = spawn_recording_server(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec(),
        ]);
        let session = Client::builder().build().session();
        let request = Request::get(format!("http://127.0.0.1:{port}/")).unwrap();
        let timed = session.send_ephemeral(&request).unwrap();
        assert_eq!(timed.response.body, b"ok");
        handle.join().unwrap();

        let mut session = Client::builder()
            .connection_policy(ConnectionPolicy::Reuse)
            .build()
            .session();
        let error = session.send_on_live_connection(&request).unwrap_err();
        assert!(matches!(error, crate::NanoGetError::Io(_)));

        let (port, _requests, handle) = spawn_recording_server(vec![
            b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nok".to_vec(),
        ]);
        let request = Request::get(format!("http://127.0.0.1:{port}/")).unwrap();
        session.ensure_connection(&request).unwrap();
        let timed = session.send_on_live_connection(&request).unwrap();
        assert_eq!(timed.response.status_code, 200);
        assert!(session.connection.is_none());
        handle.join().unwrap();
    }

    #[test]
    fn range_lookup_covers_stale_and_partial_only_if_cached_paths() {
        let mut cache = MemoryCache::default();
        let request = Request::get("http://example.com/stale-range").unwrap();
        let stale_response = response(
            200,
            vec![
                Header::unchecked("Cache-Control", "max-age=0"),
                Header::unchecked("Date", "Sun, 06 Nov 1994 08:49:37 GMT"),
                Header::unchecked("ETag", "\"stale\""),
                Header::unchecked("Content-Length", "6"),
            ],
            b"abcdef",
        );
        cache.store(
            &request,
            &timed_response(stale_response),
            &AuthContext::default(),
        );
        let mut stale_range = Request::get("http://example.com/stale-range").unwrap();
        stale_range.range_bytes(Some(0), Some(2)).unwrap();
        assert!(cache
            .lookup_range(&stale_range, SystemTime::now(), &AuthContext::default())
            .is_none());

        let mut partial_request = Request::get("http://example.com/partial-only").unwrap();
        partial_request.range_bytes(Some(0), Some(2)).unwrap();
        let partial_response = response(
            206,
            vec![
                Header::unchecked("Cache-Control", "max-age=60"),
                Header::unchecked("ETag", "\"p2\""),
                Header::unchecked("Content-Range", "bytes 0-2/6"),
                Header::unchecked("Content-Length", "3"),
            ],
            b"abc",
        );
        cache.store(
            &partial_request,
            &timed_response(partial_response),
            &AuthContext::default(),
        );

        let mut mismatch = Request::get("http://example.com/partial-only").unwrap();
        mismatch.range_bytes(Some(0), Some(1)).unwrap();
        mismatch.if_range("\"different\"").unwrap();
        mismatch
            .add_header("Cache-Control", "only-if-cached")
            .unwrap();
        assert!(matches!(
            cache.lookup_range(&mismatch, SystemTime::now(), &AuthContext::default()),
            Some(super::RangeCacheLookup::UnsatisfiedOnlyIfCached)
        ));

        let mut no_segment = Request::get("http://example.com/partial-only").unwrap();
        no_segment.range_bytes(Some(4), Some(5)).unwrap();
        assert!(cache
            .lookup_range(&no_segment, SystemTime::now(), &AuthContext::default())
            .is_none());
    }

    #[test]
    fn prepared_request_applies_proxy_headers_for_http_proxy_targets() {
        let mut proxy = ProxyConfig::new("http://127.0.0.1:8080").unwrap();
        proxy.add_header("X-Proxy", "yes").unwrap();
        let config = ClientConfig {
            proxy: Some(proxy),
            ..ClientConfig::default()
        };
        let request = Request::get("http://example.com").unwrap();
        let prepared =
            super::prepared_request(&request, &config, super::SendTarget::HttpProxy).unwrap();
        assert_eq!(prepared.header("x-proxy"), Some("yes"));

        let https_request = Request::get("https://example.com").unwrap();
        let tunnel =
            super::prepared_request(&https_request, &config, super::SendTarget::Tunnel).unwrap();
        assert_eq!(tunnel.header("x-proxy"), None);
    }

    #[test]
    fn open_connection_rejects_unknown_url_schemes() {
        let mut url = Url::parse("http://example.com").unwrap();
        url.scheme = "ws".to_string();
        let request = Request::new(Method::Get, url).unwrap();
        let error = super::open_connection(&ClientConfig::default(), &request)
            .err()
            .expect("unexpected successful connection");
        assert!(matches!(error, crate::NanoGetError::UnsupportedScheme(_)));
    }

    #[test]
    fn proxy_send_target_and_connection_keys_cover_tunnel_variants() {
        let proxy = ProxyConfig::new("http://127.0.0.1:8080").unwrap();
        let config = ClientConfig {
            proxy: Some(proxy),
            ..ClientConfig::default()
        };

        let http_request = Request::get("http://example.com").unwrap();
        assert!(matches!(
            super::SendTarget::for_request(&config, &http_request),
            super::SendTarget::HttpProxy
        ));

        let https_request = Request::get("https://example.com").unwrap();
        assert!(matches!(
            super::SendTarget::for_request(&config, &https_request),
            super::SendTarget::Tunnel
        ));

        let key = super::connection_key(&config.proxy, https_request.url());
        assert!(matches!(key, super::ConnectionKey::HttpsTunnel { .. }));
    }

    #[test]
    fn open_connection_with_proxy_https_path_reaches_tunnel_branch() {
        let proxy = ProxyConfig::new("http://127.0.0.1:9").unwrap();
        let config = ClientConfig {
            proxy: Some(proxy),
            ..ClientConfig::default()
        };
        let request = Request::get("https://example.com").unwrap();
        let error = super::open_connection(&config, &request)
            .err()
            .expect("unexpected successful connection");
        assert!(matches!(
            error,
            crate::NanoGetError::Connect(_)
                | crate::NanoGetError::Io(_)
                | crate::NanoGetError::HttpsFeatureRequired
                | crate::NanoGetError::Tls(_)
                | crate::NanoGetError::ProxyConnectFailed(_, _)
        ));
    }

    #[test]
    fn memory_cache_lookup_and_range_paths_cover_only_if_cached_and_if_range_logic() {
        let mut cache = MemoryCache::default();
        let mut only_if_cached = Request::get("http://example.com").unwrap();
        only_if_cached
            .add_header("Cache-Control", "only-if-cached")
            .unwrap();
        assert!(matches!(
            cache.lookup(&only_if_cached, SystemTime::now(), &AuthContext::default()),
            Some(super::CacheLookup::UnsatisfiedOnlyIfCached)
        ));

        let mut seeded_request = Request::get("http://example.com").unwrap();
        seeded_request.add_header("Accept", "text/plain").unwrap();
        let seeded_response = response(
            200,
            vec![
                Header::unchecked("Cache-Control", "max-age=60"),
                Header::unchecked("Vary", "Accept"),
                Header::unchecked("ETag", "\"v1\""),
            ],
            b"abcdef",
        );
        cache.store(
            &seeded_request,
            &timed_response(seeded_response),
            &AuthContext::default(),
        );

        let mut mismatched = Request::get("http://example.com").unwrap();
        mismatched.add_header("Accept", "application/json").unwrap();
        mismatched
            .add_header("Cache-Control", "only-if-cached")
            .unwrap();
        assert!(matches!(
            cache.lookup(&mismatched, SystemTime::now(), &AuthContext::default()),
            Some(super::CacheLookup::UnsatisfiedOnlyIfCached)
        ));

        let mut range_hit = Request::get("http://example.com").unwrap();
        range_hit.add_header("Accept", "text/plain").unwrap();
        range_hit.range_bytes(Some(1), Some(3)).unwrap();
        let lookup = cache.lookup_range(&range_hit, SystemTime::now(), &AuthContext::default());
        assert!(matches!(lookup, Some(super::RangeCacheLookup::Hit(_))));

        let mut range_mismatch = Request::get("http://example.com").unwrap();
        range_mismatch.add_header("Accept", "text/plain").unwrap();
        range_mismatch.range_bytes(Some(0), Some(1)).unwrap();
        range_mismatch.if_range("\"v2\"").unwrap();
        assert!(matches!(
            cache.lookup_range(&range_mismatch, SystemTime::now(), &AuthContext::default()),
            Some(super::RangeCacheLookup::IfRangeMismatch)
        ));

        let mut range_mismatch_only_cached = Request::get("http://example.com").unwrap();
        range_mismatch_only_cached
            .add_header("Accept", "text/plain")
            .unwrap();
        range_mismatch_only_cached
            .range_bytes(Some(0), Some(1))
            .unwrap();
        range_mismatch_only_cached.if_range("\"v2\"").unwrap();
        range_mismatch_only_cached
            .add_header("Cache-Control", "only-if-cached")
            .unwrap();
        assert!(matches!(
            cache.lookup_range(
                &range_mismatch_only_cached,
                SystemTime::now(),
                &AuthContext::default()
            ),
            Some(super::RangeCacheLookup::UnsatisfiedOnlyIfCached)
        ));

        let mut only_if_cached_range = Request::get("http://example.net").unwrap();
        only_if_cached_range.range_bytes(Some(0), Some(1)).unwrap();
        only_if_cached_range
            .add_header("Cache-Control", "only-if-cached")
            .unwrap();
        assert!(matches!(
            cache.lookup_range(
                &only_if_cached_range,
                SystemTime::now(),
                &AuthContext::default()
            ),
            Some(super::RangeCacheLookup::UnsatisfiedOnlyIfCached)
        ));
    }

    #[test]
    fn partial_cache_lookup_and_promotion_cover_additional_branches() {
        let mut cache = MemoryCache::default();
        let mut partial_request = Request::get("http://example.com/partial").unwrap();
        partial_request.range_bytes(Some(0), Some(2)).unwrap();
        let partial_response = response(
            206,
            vec![
                Header::unchecked("Cache-Control", "max-age=60"),
                Header::unchecked("ETag", "\"part\""),
                Header::unchecked("Content-Range", "bytes 0-2/6"),
                Header::unchecked("Content-Length", "3"),
            ],
            b"abc",
        );
        cache.store(
            &partial_request,
            &timed_response(partial_response),
            &AuthContext::default(),
        );

        let mut hit_request = Request::get("http://example.com/partial").unwrap();
        hit_request.range_bytes(Some(1), Some(2)).unwrap();
        let lookup = cache.lookup_range(&hit_request, SystemTime::now(), &AuthContext::default());
        assert!(matches!(lookup, Some(super::RangeCacheLookup::Hit(_))));

        let mut miss_request = Request::get("http://example.com/partial").unwrap();
        miss_request.range_bytes(Some(4), Some(5)).unwrap();
        assert!(cache
            .lookup_range(&miss_request, SystemTime::now(), &AuthContext::default())
            .is_none());

        let mut mismatch = Request::get("http://example.com/partial").unwrap();
        mismatch.range_bytes(Some(0), Some(1)).unwrap();
        mismatch.if_range("\"different\"").unwrap();
        assert!(matches!(
            cache.lookup_range(&mismatch, SystemTime::now(), &AuthContext::default()),
            Some(super::RangeCacheLookup::IfRangeMismatch)
        ));

        let invalid_partial = response(
            206,
            vec![
                Header::unchecked("Cache-Control", "max-age=60"),
                Header::unchecked("ETag", "\"part\""),
            ],
            b"abc",
        );
        cache.store(
            &partial_request,
            &timed_response(invalid_partial),
            &AuthContext::default(),
        );
    }

    #[test]
    fn cache_entry_and_partial_entry_helpers_cover_unhit_branches() {
        let request = Request::get("http://example.com/cache").unwrap();
        let base_response = response(
            200,
            vec![
                Header::unchecked("Cache-Control", "max-age=2"),
                Header::unchecked("Date", "Sun, 06 Nov 1994 08:49:37 GMT"),
                Header::unchecked("ETag", "\"v1\""),
                Header::unchecked("Content-Length", "6"),
            ],
            b"abcdef",
        );
        let mut entry = CacheEntry::new(
            &request,
            &timed_response(base_response),
            AuthContext::default(),
        )
        .unwrap();
        entry.cache_control.no_cache = true;
        assert!(!entry.is_fresh(SystemTime::now()));
        entry.cache_control.no_cache = false;
        entry.date_header = Some(entry.response_time + Duration::from_secs(1));
        let now = entry.response_time + Duration::from_secs(3);
        let _ = entry.current_age(now);
        let _ = entry.remaining_freshness(now);
        let _ = entry.staleness(now);
        assert!(!entry.satisfies_request(
            &CacheControl {
                max_age: Some(1),
                ..CacheControl::default()
            },
            now
        ));
        assert!(!entry.satisfies_request(
            &CacheControl {
                min_fresh: Some(99),
                ..CacheControl::default()
            },
            now
        ));
        entry.freshness_lifetime = Duration::from_secs(0);
        assert!(entry.satisfies_request(
            &CacheControl {
                max_stale: Some(Some(10)),
                ..CacheControl::default()
            },
            now
        ));

        let partial_response = response(
            206,
            vec![
                Header::unchecked("Cache-Control", "max-age=1"),
                Header::unchecked("ETag", "\"p1\""),
                Header::unchecked("Date", "Sun, 06 Nov 1994 08:49:37 GMT"),
                Header::unchecked("Content-Range", "bytes 0-2/3"),
                Header::unchecked("Content-Length", "3"),
            ],
            b"abc",
        );
        let mut partial = PartialCacheEntry::new(
            &request,
            &timed_response(partial_response),
            AuthContext::default(),
        )
        .unwrap();
        partial.date_header = Some(partial.response_time + Duration::from_secs(1));
        let now = partial.response_time + Duration::from_secs(2);
        let _ = partial.current_age(now);
        let _ = partial.remaining_freshness(now);
        let _ = partial.staleness(now);
        partial.cache_control.no_cache = true;
        assert!(!partial.is_fresh(now));
        assert!(!partial.satisfies_request(&CacheControl::default(), now));
        partial.cache_control.no_cache = false;
        assert!(!partial.satisfies_request(
            &CacheControl {
                max_age: Some(0),
                ..CacheControl::default()
            },
            now
        ));
        assert!(!partial.satisfies_request(
            &CacheControl {
                min_fresh: Some(100),
                ..CacheControl::default()
            },
            now
        ));
        partial.freshness_lifetime = Duration::from_secs(0);
        assert!(partial.satisfies_request(
            &CacheControl {
                max_stale: Some(Some(10)),
                ..CacheControl::default()
            },
            now
        ));
        let range = partial
            .range_response(
                ByteRange {
                    start: Some(1),
                    end: Some(2),
                },
                now,
            )
            .unwrap();
        assert_eq!(range.status_code, 206);
        assert_eq!(range.body, b"bc");

        partial.segments.push(ByteSegment {
            start: 10,
            end: 11,
            bytes: b"zz".to_vec(),
        });
        assert!(partial.promote_complete().is_none());
    }

    #[test]
    fn cache_satisfies_request_specific_branches_are_exercised() {
        let request = Request::get("http://example.com/age").unwrap();
        let base = response(
            200,
            vec![
                Header::unchecked("Cache-Control", "max-age=100"),
                Header::unchecked("Content-Length", "1"),
            ],
            b"x",
        );
        let mut entry =
            CacheEntry::new(&request, &timed_response(base), AuthContext::default()).unwrap();
        entry.request_time = UNIX_EPOCH;
        entry.response_time = UNIX_EPOCH + Duration::from_secs(10);
        entry.date_header = Some(UNIX_EPOCH);
        let now = UNIX_EPOCH + Duration::from_secs(20);
        assert!(!entry.satisfies_request(
            &CacheControl {
                max_age: Some(1),
                ..CacheControl::default()
            },
            now
        ));
        assert!(!entry.satisfies_request(
            &CacheControl {
                min_fresh: Some(200),
                ..CacheControl::default()
            },
            now
        ));

        let partial_response = response(
            206,
            vec![
                Header::unchecked("Cache-Control", "max-age=100"),
                Header::unchecked("ETag", "\"p-age\""),
                Header::unchecked("Content-Range", "bytes 0-0/1"),
                Header::unchecked("Content-Length", "1"),
            ],
            b"x",
        );
        let mut partial = PartialCacheEntry::new(
            &request,
            &timed_response(partial_response),
            AuthContext::default(),
        )
        .unwrap();
        partial.request_time = UNIX_EPOCH;
        partial.response_time = UNIX_EPOCH + Duration::from_secs(10);
        partial.date_header = Some(UNIX_EPOCH);
        let now = UNIX_EPOCH + Duration::from_secs(20);
        assert!(!partial.satisfies_request(
            &CacheControl {
                max_age: Some(1),
                ..CacheControl::default()
            },
            now
        ));
        assert!(!partial.satisfies_request(
            &CacheControl {
                min_fresh: Some(200),
                ..CacheControl::default()
            },
            now
        ));
        partial.freshness_lifetime = Duration::from_secs(0);
        partial.cache_control.must_revalidate = true;
        assert!(!partial.satisfies_request(
            &CacheControl {
                max_stale: Some(None),
                ..CacheControl::default()
            },
            now
        ));
        partial.cache_control.must_revalidate = false;
        assert!(partial.satisfies_request(
            &CacheControl {
                max_stale: Some(None),
                ..CacheControl::default()
            },
            now
        ));
        assert!(!partial.satisfies_request(&CacheControl::default(), now));
    }

    #[test]
    fn cache_construction_rejections_and_misc_helpers_cover_branches() {
        let request = Request::get("http://example.com/path").unwrap();
        let now = SystemTime::now();

        let no_store = response(
            200,
            vec![Header::unchecked("Cache-Control", "no-store")],
            b"x",
        );
        assert!(
            CacheEntry::new(&request, &timed_response(no_store), AuthContext::default()).is_none()
        );

        let cacheable = response(
            200,
            vec![Header::unchecked("Cache-Control", "max-age=60")],
            b"x",
        );
        assert!(CacheEntry::new(
            &request,
            &timed_response(cacheable.clone()),
            AuthContext {
                origin: None,
                proxy: Some("p".to_string())
            }
        )
        .is_none());

        let no_store_partial = response(
            206,
            vec![
                Header::unchecked("Cache-Control", "no-store"),
                Header::unchecked("ETag", "\"v1\""),
                Header::unchecked("Content-Range", "bytes 0-0/1"),
                Header::unchecked("Content-Length", "1"),
            ],
            b"x",
        );
        assert!(PartialCacheEntry::new(
            &request,
            &timed_response(no_store_partial),
            AuthContext::default()
        )
        .is_none());

        let proxy_partial = response(
            206,
            vec![
                Header::unchecked("Cache-Control", "max-age=60"),
                Header::unchecked("ETag", "\"v1\""),
                Header::unchecked("Content-Range", "bytes 0-0/1"),
                Header::unchecked("Content-Length", "1"),
            ],
            b"x",
        );
        assert!(PartialCacheEntry::new(
            &request,
            &timed_response(proxy_partial.clone()),
            AuthContext {
                origin: None,
                proxy: Some("p".to_string())
            }
        )
        .is_none());
        assert!(PartialCacheEntry::new(
            &request,
            &timed_response(proxy_partial.clone()),
            AuthContext {
                origin: Some("secret".to_string()),
                proxy: None
            }
        )
        .is_none());

        let invalid_range = response(
            206,
            vec![
                Header::unchecked("Cache-Control", "max-age=60"),
                Header::unchecked("ETag", "\"v1\""),
                Header::unchecked("Content-Range", "bytes 0-4/10"),
                Header::unchecked("Content-Length", "5"),
            ],
            b"abc",
        );
        assert!(PartialCacheEntry::new(
            &request,
            &timed_response(invalid_range),
            AuthContext::default()
        )
        .is_none());

        let weak_etag = response(
            206,
            vec![
                Header::unchecked("Cache-Control", "max-age=60"),
                Header::unchecked("ETag", "W/\"v1\""),
                Header::unchecked("Content-Range", "bytes 0-2/3"),
                Header::unchecked("Content-Length", "3"),
            ],
            b"abc",
        );
        assert!(PartialCacheEntry::new(
            &request,
            &timed_response(weak_etag),
            AuthContext::default()
        )
        .is_none());

        let mut cache = MemoryCache::default();
        let mut get = Request::get("http://example.com/upsert").unwrap();
        get.add_header("Accept", "text/plain").unwrap();
        let first = response(
            200,
            vec![
                Header::unchecked("Cache-Control", "max-age=60"),
                Header::unchecked("Vary", "Accept"),
                Header::unchecked("Content-Length", "1"),
            ],
            b"a",
        );
        let second = response(
            200,
            vec![
                Header::unchecked("Cache-Control", "max-age=60"),
                Header::unchecked("Vary", "Accept"),
                Header::unchecked("Content-Length", "1"),
            ],
            b"b",
        );
        let first_entry =
            CacheEntry::new(&get, &timed_response(first), AuthContext::default()).unwrap();
        let second_entry =
            CacheEntry::new(&get, &timed_response(second), AuthContext::default()).unwrap();
        cache.upsert_complete_entry(get.url().cache_key(), first_entry);
        cache.upsert_complete_entry(get.url().cache_key(), second_entry);
        assert_eq!(cache.entries[&get.url().cache_key()].len(), 1);

        let direct = CacheControl::from_headers(&[Header::unchecked("Cache-Control", "x-test=1")]);
        assert!(!direct.no_store);
        let head_response = super::response_for_method(
            &cache.entries[&get.url().cache_key()][0].response,
            Method::Head,
        );
        assert!(head_response.body.is_empty());

        let vary_star_response = response(200, vec![Header::unchecked("Vary", "*")], b"");
        assert!(super::extract_vary_headers(&request, &vary_star_response).is_none());

        assert!(super::is_cacheable_status(203));
        assert!(!super::is_cacheable_status(418));

        let expires = response(
            200,
            vec![
                Header::unchecked("Date", "Sun, 06 Nov 1994 08:49:37 GMT"),
                Header::unchecked("Expires", "Sun, 06 Nov 1994 08:50:37 GMT"),
            ],
            b"",
        );
        assert_eq!(
            super::compute_freshness_lifetime(&expires, now),
            Duration::from_secs(60)
        );

        let heuristic = response(
            200,
            vec![
                Header::unchecked("Date", "Sun, 06 Nov 1994 08:49:37 GMT"),
                Header::unchecked("Last-Modified", "Sun, 06 Nov 1994 08:39:37 GMT"),
            ],
            b"",
        );
        assert_eq!(
            super::compute_freshness_lifetime(&heuristic, now),
            Duration::from_secs(60)
        );

        let expires_without_date = response(
            200,
            vec![Header::unchecked(
                "Expires",
                "Sun, 06 Nov 1994 08:50:37 GMT",
            )],
            b"",
        );
        assert_eq!(
            super::compute_freshness_lifetime(
                &expires_without_date,
                UNIX_EPOCH + Duration::from_secs(784_111_700)
            ),
            Duration::from_secs(137)
        );

        let existing = CacheEntry::new(
            &request,
            &timed_response(response(
                200,
                vec![
                    Header::unchecked("Cache-Control", "max-age=60"),
                    Header::unchecked("ETag", "\"v1\""),
                    Header::unchecked("Content-Length", "3"),
                ],
                b"abc",
            )),
            AuthContext::default(),
        )
        .unwrap();
        let different_etag = CacheEntry::new(
            &request,
            &timed_response(response(
                200,
                vec![
                    Header::unchecked("Cache-Control", "max-age=60"),
                    Header::unchecked("ETag", "\"v2\""),
                    Header::unchecked("Content-Length", "3"),
                ],
                b"abc",
            )),
            AuthContext::default(),
        )
        .unwrap();
        assert!(!super::head_update_is_compatible(
            &existing,
            &different_etag
        ));
        let different_length = CacheEntry::new(
            &request,
            &timed_response(response(
                200,
                vec![
                    Header::unchecked("Cache-Control", "max-age=60"),
                    Header::unchecked("ETag", "\"v1\""),
                    Header::unchecked("Content-Length", "4"),
                ],
                b"abcd",
            )),
            AuthContext::default(),
        )
        .unwrap();
        assert!(!super::head_update_is_compatible(
            &existing,
            &different_length
        ));
        let same_length = CacheEntry::new(
            &request,
            &timed_response(response(
                200,
                vec![
                    Header::unchecked("Cache-Control", "max-age=60"),
                    Header::unchecked("ETag", "\"v1\""),
                    Header::unchecked("Content-Length", "3"),
                ],
                b"abc",
            )),
            AuthContext::default(),
        )
        .unwrap();
        assert!(super::head_update_is_compatible(&existing, &same_length));

        assert!(!super::if_range_matches_entry(
            Some("invalid-date"),
            None,
            Some("Sun, 06 Nov 1994 08:49:37 GMT")
        ));
        assert!(!super::if_range_matches_entry(
            Some("Sun, 06 Nov 1994 08:49:37 GMT"),
            None,
            None
        ));
    }

    #[test]
    fn authentication_retry_helper_covers_remaining_branches() {
        let request = Request::get("http://example.com").unwrap();
        let mut seen = Vec::new();
        let empty_challenge_response =
            response(401, vec![Header::unchecked("WWW-Authenticate", "")], b"");
        assert!(super::maybe_retry_request_auth(
            None,
            AuthTarget::Origin,
            &request,
            &empty_challenge_response,
            &mut seen
        )
        .unwrap()
        .is_none());

        let challenge_response = response(
            401,
            vec![Header::unchecked("WWW-Authenticate", "Basic realm=\"api\"")],
            b"",
        );
        assert!(super::maybe_retry_request_auth(
            None,
            AuthTarget::Origin,
            &request,
            &challenge_response,
            &mut seen
        )
        .unwrap()
        .is_none());
        assert!(super::maybe_retry_request_auth(
            None,
            AuthTarget::Origin,
            &request,
            &challenge_response,
            &mut seen
        )
        .unwrap()
        .is_none());

        let no_match_handler: Arc<dyn AuthHandler + Send + Sync> = Arc::new(NoMatchAuthHandler);
        assert!(super::maybe_retry_request_auth(
            Some(&no_match_handler),
            AuthTarget::Origin,
            &request,
            &challenge_response,
            &mut Vec::new()
        )
        .unwrap()
        .is_none());

        let use_headers_handler: Arc<dyn AuthHandler + Send + Sync> =
            Arc::new(UseHeadersAuthHandler {
                header_name: "Authorization",
                header_value: "Basic dXNlcjpwYXNz",
            });
        let retried = super::maybe_retry_request_auth(
            Some(&use_headers_handler),
            AuthTarget::Origin,
            &request,
            &challenge_response,
            &mut Vec::new(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(retried.header("authorization"), Some("Basic dXNlcjpwYXNz"));

        let abort_handler: Arc<dyn AuthHandler + Send + Sync> = Arc::new(AbortAuthHandler);
        let proxy_response = response(
            407,
            vec![Header::unchecked(
                "Proxy-Authenticate",
                "Basic realm=\"proxy\"",
            )],
            b"",
        );
        let error = super::maybe_retry_request_auth(
            Some(&abort_handler),
            AuthTarget::Proxy,
            &request,
            &proxy_response,
            &mut Vec::new(),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            crate::NanoGetError::AuthenticationRejected(message)
            if message.contains("proxy authentication handler aborted")
        ));
    }

    #[test]
    fn proxy_helper_paths_cover_remaining_validation_and_headers() {
        assert!(matches!(
            super::validate_proxy_header_name("Host"),
            Err(crate::NanoGetError::ProtocolManagedHeader(_))
        ));
        assert!(matches!(
            super::validate_proxy_header_name("TE"),
            Err(crate::NanoGetError::HopByHopHeader(_))
        ));

        let proxy = ProxyConfig::new("http://127.0.0.1:8080").unwrap();
        let request = Request::get("https://example.com").unwrap();
        let config = ClientConfig {
            preemptive_proxy_authorization: Some("Basic cHJveHk6c2VjcmV0".to_string()),
            ..ClientConfig::default()
        };
        let headers = prepared_connect_headers(&request, &config, &proxy).unwrap();
        assert!(headers
            .iter()
            .any(|header| header.matches_name("proxy-authorization")));

        let mut request_with_proxy = Request::get("https://example.com").unwrap();
        request_with_proxy
            .proxy_authorization("Basic explicit")
            .unwrap();
        let headers = prepared_connect_headers(&request_with_proxy, &config, &proxy).unwrap();
        assert!(headers
            .iter()
            .any(|header| header.value() == "Basic explicit"));

        let mut cache = MemoryCache::default();
        let get_request = Request::get("http://example.com/head").unwrap();
        let get_response = response(
            200,
            vec![
                Header::unchecked("Cache-Control", "max-age=60"),
                Header::unchecked("ETag", "\"v1\""),
                Header::unchecked("Content-Length", "6"),
            ],
            b"abcdef",
        );
        cache.store(
            &get_request,
            &timed_response(get_response),
            &AuthContext::default(),
        );
        let head_request = Request::head("http://example.com/head").unwrap();
        let head_response = response(
            200,
            vec![
                Header::unchecked("Cache-Control", "max-age=60"),
                Header::unchecked("ETag", "\"v2\""),
                Header::unchecked("Content-Length", "8"),
            ],
            b"",
        );
        cache.store(
            &head_request,
            &timed_response(head_response),
            &AuthContext::default(),
        );
    }

    #[cfg(not(feature = "https"))]
    #[test]
    fn open_https_tunnel_reports_feature_required_after_connect_success() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 1024];
            let _ = stream.read(&mut buffer).unwrap();
            stream
                .write_all(b"HTTP/1.1 200 Connection Established\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
        });

        let proxy = ProxyConfig::new(format!("http://127.0.0.1:{port}")).unwrap();
        let config = ClientConfig::default();
        let request = Request::get("https://example.com").unwrap();
        let error = super::open_https_tunnel(&config, &request, &proxy)
            .err()
            .expect("expected tunnel failure");
        assert!(matches!(error, crate::NanoGetError::HttpsFeatureRequired));
        handle.join().unwrap();
    }

    #[cfg(not(feature = "https"))]
    #[test]
    fn open_https_tunnel_without_auth_handler_returns_proxy_connect_failed() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 1024];
            let _ = stream.read(&mut buffer).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"proxy\"\r\nContent-Length: 0\r\n\r\n",
                )
                .unwrap();
        });

        let proxy = ProxyConfig::new(format!("http://127.0.0.1:{port}")).unwrap();
        let request = Request::get("https://example.com").unwrap();
        let error = super::open_https_tunnel(&ClientConfig::default(), &request, &proxy)
            .err()
            .expect("expected proxy failure");
        assert!(matches!(
            error,
            crate::NanoGetError::ProxyConnectFailed(407, _)
        ));
        handle.join().unwrap();
    }

    #[cfg(not(feature = "https"))]
    #[test]
    fn open_https_tunnel_non_auth_proxy_failure_is_reported() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 1024];
            let _ = stream.read(&mut buffer).unwrap();
            stream
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
        });

        let proxy = ProxyConfig::new(format!("http://127.0.0.1:{port}")).unwrap();
        let request = Request::get("https://example.com").unwrap();
        let error = super::open_https_tunnel(&ClientConfig::default(), &request, &proxy)
            .err()
            .expect("expected proxy failure");
        assert!(matches!(
            error,
            crate::NanoGetError::ProxyConnectFailed(502, _)
        ));
        handle.join().unwrap();
    }

    #[cfg(feature = "https")]
    #[test]
    fn open_https_tunnel_returns_proxy_error_without_handler() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 1024];
            let _ = stream.read(&mut buffer).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"proxy\"\r\nContent-Length: 0\r\n\r\n",
                )
                .unwrap();
        });

        let proxy = ProxyConfig::new(format!("http://127.0.0.1:{port}")).unwrap();
        let request = Request::get("https://example.com").unwrap();
        let error = super::open_https_tunnel(&ClientConfig::default(), &request, &proxy)
            .err()
            .expect("expected tunnel failure");
        assert!(matches!(
            error,
            crate::NanoGetError::ProxyConnectFailed(407, _)
        ));
        handle.join().unwrap();
    }

    #[cfg(not(feature = "https"))]
    #[test]
    fn open_https_tunnel_retries_proxy_auth_challenges() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let requests_for_thread = Arc::clone(&requests);
        let handle = thread::spawn(move || {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut chunk = [0u8; 256];
                let mut read = stream.read(&mut chunk).unwrap();
                while read > 0 {
                    request.extend_from_slice(&chunk[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                    read = stream.read(&mut chunk).unwrap();
                }
                requests_for_thread
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&request).into_owned());
                if index == 0 {
                    stream
                        .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"proxy\"\r\nContent-Length: 0\r\n\r\n")
                        .unwrap();
                } else {
                    stream
                        .write_all(
                            b"HTTP/1.1 200 Connection Established\r\nContent-Length: 0\r\n\r\n",
                        )
                        .unwrap();
                }
            }
        });

        let proxy = ProxyConfig::new(format!("http://127.0.0.1:{port}")).unwrap();
        let config = ClientConfig {
            proxy_auth_handler: Some(Arc::new(super::BasicAuthHandler::new(
                "proxy",
                "secret",
                AuthTarget::Proxy,
            ))),
            ..ClientConfig::default()
        };
        let request = Request::get("https://example.com").unwrap();
        let error = super::open_https_tunnel(&config, &request, &proxy)
            .err()
            .expect("expected tunnel failure");
        assert!(matches!(error, crate::NanoGetError::HttpsFeatureRequired));

        let captured = requests.lock().unwrap().clone();
        assert_eq!(captured.len(), 2);
        assert!(!captured[0].contains("Proxy-Authorization:"));
        assert!(captured[1].contains("Proxy-Authorization: Basic"));
        handle.join().unwrap();
    }

    #[cfg(feature = "https")]
    #[test]
    fn open_https_tunnel_retries_proxy_auth_before_reporting_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::<String>::new()));
        let requests_for_thread = Arc::clone(&requests);
        let handle = thread::spawn(move || {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut chunk = [0u8; 256];
                let mut read = stream.read(&mut chunk).unwrap();
                while read > 0 {
                    request.extend_from_slice(&chunk[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                    read = stream.read(&mut chunk).unwrap();
                }
                requests_for_thread
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&request).into_owned());
                if index == 0 {
                    stream
                        .write_all(
                            b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"proxy\"\r\nContent-Length: 0\r\n\r\n",
                        )
                        .unwrap();
                } else {
                    stream
                        .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                        .unwrap();
                }
            }
        });

        let proxy = ProxyConfig::new(format!("http://127.0.0.1:{port}")).unwrap();
        let config = ClientConfig {
            proxy_auth_handler: Some(Arc::new(super::BasicAuthHandler::new(
                "proxy",
                "secret",
                AuthTarget::Proxy,
            ))),
            ..ClientConfig::default()
        };
        let request = Request::get("https://example.com").unwrap();
        let error = super::open_https_tunnel(&config, &request, &proxy)
            .err()
            .expect("expected https feature error");
        assert!(matches!(
            error,
            crate::NanoGetError::ProxyConnectFailed(502, _)
        ));

        let captured = requests.lock().unwrap().clone();
        assert_eq!(captured.len(), 2);
        assert!(!captured[0].contains("Proxy-Authorization:"));
        assert!(captured[1].contains("Proxy-Authorization: Basic"));
        handle.join().unwrap();
    }
}
