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
                crate::response::read_parsed_response(&mut connection.reader, request.method())?
            };
            if parsed.connection_close && index + 1 != requests.len() {
                self.connection = None;
                return Err(NanoGetError::Pipeline(
                    "server closed the connection before the pipeline drained".to_string(),
                ));
            }

            responses.push(parsed.response);
            if parsed.connection_close {
                self.connection = None;
            }
        }

        Ok(responses)
    }

    fn execute_one(&mut self, request: Request) -> Result<Response, NanoGetError> {
        let auth_context = effective_auth_context(&request, &self.config);
        let cache_directives = CacheControl::from_headers(request.headers());
        if self.config.cache_mode != CacheMode::Memory && cache_directives.only_if_cached {
            return Ok(gateway_timeout_response());
        }

        let cache_lookup = if self.config.cache_mode == CacheMode::Memory
            && request.method() == Method::Get
            && !cache_directives.no_store
        {
            self.cache
                .lock()
                .map_err(|_| NanoGetError::Cache("cache lock poisoned".to_string()))?
                .lookup(&request, SystemTime::now(), &auth_context)
        } else {
            None
        };

        match cache_lookup {
            Some(CacheLookup::Fresh(response)) => {
                return Ok(response_for_method(&response, request.method()))
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
        let parsed =
            crate::response::read_parsed_response(&mut connection.reader, request.method())?;
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
        let parsed =
            crate::response::read_parsed_response(&mut connection.reader, request.method())?;
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

    if let Some(proxy) = &config.proxy {
        if send_target.uses_proxy() {
            for header in proxy.headers() {
                if !prepared.has_header(header.name()) {
                    prepared.add_header(header.name().to_string(), header.value().to_string())?;
                }
            }
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
        http::write_connect_request(
            &mut stream,
            &request.url().authority_form(),
            &connect_headers,
            false,
        )?;
        use std::io::Write;
        stream.flush()?;
        let head = http::read_response_head(&mut stream)?;
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

        if head.status_code == 407 {
            let response = Response {
                version: head.version,
                status_code: head.status_code,
                reason_phrase: head.reason_phrase.clone(),
                headers: head.headers,
                trailers: Vec::new(),
                body: Vec::new(),
            };
            if let Some(retry) = maybe_retry_request_auth(
                config.proxy_auth_handler.as_ref(),
                AuthTarget::Proxy,
                &current,
                &response,
                &mut seen_proxy_challenges,
            )? {
                current = retry;
                continue;
            }
        }

        return Err(NanoGetError::ProxyConnectFailed(
            head.status_code,
            head.reason_phrase,
        ));
    }
}

#[derive(Default)]
struct MemoryCache {
    entries: HashMap<String, Vec<CacheEntry>>,
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
            .find(|entry| entry.matches(request, auth_context))
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

        Some(CacheLookup::Fresh(entry.response.clone()))
    }

    fn store(&mut self, request: &Request, response: &TimedResponse, auth_context: &AuthContext) {
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
                let body = existing.response.body.clone();
                let mut updated = entry;
                updated.response.body = body;
                *existing = updated;
            } else {
                *existing = entry;
            }
            return;
        }

        variants.push(entry);
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

        merge_headers(&mut existing.response.headers, &not_modified.headers);
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
        if let Some(max_age) = request_cache_control.max_age {
            if age > Duration::from_secs(max_age) {
                return false;
            }
        }

        if let Some(min_fresh) = request_cache_control.min_fresh {
            if self.remaining_freshness(now) < Duration::from_secs(min_fresh) {
                return false;
            }
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
}

enum CacheLookup {
    Fresh(Response),
    Stale(Box<CacheEntry>),
    UnsatisfiedOnlyIfCached,
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
                if directive.eq_ignore_ascii_case("no-store") {
                    directives.no_store = true;
                } else if directive.eq_ignore_ascii_case("no-cache") {
                    directives.no_cache = true;
                } else if directive.eq_ignore_ascii_case("must-revalidate") {
                    directives.must_revalidate = true;
                } else if directive.eq_ignore_ascii_case("proxy-revalidate") {
                    directives.proxy_revalidate = true;
                } else if directive.eq_ignore_ascii_case("public") {
                    directives.public = true;
                } else if directive.eq_ignore_ascii_case("private") {
                    directives.private = true;
                } else if directive.eq_ignore_ascii_case("only-if-cached") {
                    directives.only_if_cached = true;
                } else if let Some(max_age) = directive.strip_prefix("max-age=") {
                    directives.max_age = max_age.parse().ok();
                } else if directive.eq_ignore_ascii_case("max-stale") {
                    directives.max_stale = Some(None);
                } else if let Some(max_stale) = directive.strip_prefix("max-stale=") {
                    directives.max_stale = max_stale.parse().ok().map(Some);
                } else if let Some(min_fresh) = directive.strip_prefix("min-fresh=") {
                    directives.min_fresh = min_fresh.parse().ok();
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
    let Some(vary) = response.header("vary") else {
        return Some(Vec::new());
    };
    if vary.split(',').any(|value| value.trim() == "*") {
        return None;
    }

    Some(
        vary.split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|name| VaryHeader {
                name: name.to_ascii_lowercase(),
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
        if let Ok(age) = date.duration_since(last_modified) {
            let heuristic = age / 10;
            return heuristic.min(Duration::from_secs(86_400));
        }
    }

    Duration::from_secs(0)
}

fn header_value<'a>(headers: &'a [Header], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|header| header.matches_name(name))
        .map(Header::value)
}

fn merge_headers(stored: &mut Vec<Header>, fresh: &[Header]) {
    for header in fresh {
        stored.retain(|existing| !existing.matches_name(header.name()));
        stored.push(header.clone());
    }
}

fn parse_age_header(headers: &[Header]) -> Option<Duration> {
    header_value(headers, "age")
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
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

    if !headers
        .iter()
        .any(|header| header.matches_name("proxy-authorization"))
    {
        if let Some(value) = request.header("proxy-authorization") {
            headers.push(Header::new("Proxy-Authorization", value.to_string())?);
        } else if let Some(value) = &config.preemptive_proxy_authorization {
            headers.push(Header::new("Proxy-Authorization", value.clone())?);
        }
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
    use std::time::{Duration, UNIX_EPOCH};

    use super::{CacheControl, CacheMode, Client, ClientBuilder, ConnectionPolicy, ProxyConfig};
    use crate::request::{Method, RedirectPolicy, Request};

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
}
