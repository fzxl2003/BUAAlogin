use axum::{
    body::Body,
    extract::{FromRequest, Path, State},
    http::{header, HeaderMap, HeaderValue, Request, StatusCode},
    middleware::{self, Next},
    response::{sse::Event as SseEvent, IntoResponse, Response, Sse},
    routing::{get, post, put},
    Json, Router,
};
use buaa_core::engine::TestKind;
use serde::{de::DeserializeOwned, Deserialize};
use std::{
    collections::HashMap,
    convert::Infallible,
    sync::{Arc, Mutex},
};
use tokio::sync::broadcast;
use tokio_stream::{wrappers::BroadcastStream, StreamExt};
use zeroize::Zeroizing;

use crate::{
    auth::{AuthState, LoginError},
    storage::{Account, Storage},
    supervisor::{notify, Model, Supervisor},
};

#[derive(Clone)]
pub struct AppState {
    pub storage: Storage,
    pub model: Arc<Mutex<Model>>,
    pub auth: Arc<Mutex<AuthState>>,
    pub supervisor: Supervisor,
    pub events: broadcast::Sender<String>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/assets/app.css", get(css))
        .route("/assets/app.js", get(javascript))
        .route("/healthz", get(health))
        .route("/api/bootstrap", get(bootstrap))
        .route("/api/setup", post(setup))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/session", get(session))
        .route("/api/state", get(state_snapshot))
        .route("/api/events", get(events))
        .route("/api/account", put(save_account))
        .route("/api/settings", put(save_settings))
        .route("/api/interfaces", put(save_interfaces))
        .route("/api/admin-password", put(change_admin_password))
        .route("/api/actions/{action}", post(action))
        .route("/api/tests/{kind}", post(test))
        .route("/api/logs/download", get(download_log))
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}

async fn security_headers(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'self'; connect-src 'self'; img-src 'self' data:; style-src 'self'; script-src 'self'; form-action 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'none'"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}

async fn index() -> Response {
    asset(
        include_str!("static/index.html"),
        "text/html; charset=utf-8",
    )
}

async fn css() -> Response {
    asset(include_str!("static/app.css"), "text/css; charset=utf-8")
}

async fn javascript() -> Response {
    asset(
        include_str!("static/app.js"),
        "text/javascript; charset=utf-8",
    )
}

fn asset(content: &'static str, content_type: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        ],
        content,
    )
        .into_response()
}

async fn health(State(state): State<AppState>) -> Response {
    let data = state.model.lock().unwrap();
    Json(serde_json::json!({
        "ok": true,
        "monitoring": data.state.monitoring,
        "phase": data.state.phase,
    }))
    .into_response()
}

async fn bootstrap(State(state): State<AppState>) -> Response {
    let auth = state.auth.lock().unwrap();
    Json(serde_json::json!({"needs_setup": auth.needs_setup()})).into_response()
}

#[derive(Deserialize)]
struct SetupInput {
    token: String,
    password: String,
}

async fn setup(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(input): ApiJson<SetupInput>,
) -> Result<Response, ApiError> {
    verify_origin(&headers)?;
    let (session, csrf) = {
        let mut auth = state.auth.lock().unwrap();
        let hash = auth
            .prepare_setup(&input.token, &input.password)
            .map_err(ApiError::bad_request)?;
        state
            .storage
            .write_auth_hash(&hash)
            .map_err(ApiError::internal)?;
        auth.commit_setup(hash).map_err(ApiError::conflict)?
    };
    Ok(auth_response(session, csrf))
}

#[derive(Deserialize)]
struct LoginInput {
    password: String,
}

async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(input): ApiJson<LoginInput>,
) -> Result<Response, ApiError> {
    verify_origin(&headers)?;
    let result = state.auth.lock().unwrap().login(&input.password);
    match result {
        Ok((session, csrf)) => Ok(auth_response(session, csrf)),
        Err(LoginError::Invalid) => Err(ApiError::new(StatusCode::UNAUTHORIZED, "管理密码错误")),
        Err(LoginError::Limited) => Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "登录失败次数过多，请五分钟后再试",
        )),
    }
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    let context = authenticate(&state, &headers)?;
    verify_write(&headers, &context.csrf)?;
    state.auth.lock().unwrap().logout(&context.session);
    let mut response = Json(serde_json::json!({"ok": true})).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("buaa_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"),
    );
    Ok(response)
}

async fn session(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    let context = authenticate(&state, &headers)?;
    Ok(Json(serde_json::json!({"csrf": context.csrf})).into_response())
}

async fn state_snapshot(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    authenticate(&state, &headers)?;
    let snapshot = state.model.lock().unwrap().state.clone();
    Ok(Json(snapshot).into_response())
}

async fn events(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    authenticate(&state, &headers)?;
    let stream = BroadcastStream::new(state.events.subscribe()).filter_map(|event| match event {
        Ok(data) => Some(Ok::<SseEvent, Infallible>(
            SseEvent::default().event("state").data(data),
        )),
        Err(_) => Some(Ok(SseEvent::default().event("refresh").data("{}"))),
    });
    Ok(Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response())
}

#[derive(Deserialize)]
struct AccountInput {
    username: String,
    password: Option<String>,
    remember: bool,
}

async fn save_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(input): ApiJson<AccountInput>,
) -> Result<Response, ApiError> {
    authorize_write(&state, &headers)?;
    let mut data = state.model.lock().unwrap();
    if data.state.monitoring || data.state.busy {
        return Err(ApiError::conflict("请先停止当前任务再修改账号"));
    }
    let username = input.username.trim();
    if username.is_empty() {
        return Err(ApiError::bad_request("账号不能为空"));
    }
    let password = input
        .password
        .filter(|value| !value.is_empty())
        .or_else(|| {
            data.account
                .as_ref()
                .map(|value| value.password.to_string())
        })
        .ok_or_else(|| ApiError::bad_request("密码不能为空"))?;
    let account = Account {
        username: username.into(),
        password: Zeroizing::new(password),
        remembered: input.remember,
    };
    state
        .storage
        .save_account(&account)
        .map_err(ApiError::internal)?;
    data.account = Some(account);
    data.refresh_account_summary();
    notify(&mut data, &state.events);
    Ok(Json(serde_json::json!({"ok": true})).into_response())
}

#[derive(Deserialize)]
struct SettingsInput {
    interval: u64,
    traffic_total_gb: String,
    portal_auth_enabled: bool,
    connectivity_probe_enabled: bool,
}

async fn save_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(input): ApiJson<SettingsInput>,
) -> Result<Response, ApiError> {
    authorize_write(&state, &headers)?;
    {
        let data = state.model.lock().unwrap();
        if (data.state.monitoring || data.state.busy)
            && (data.state.settings.portal_auth_enabled != input.portal_auth_enabled
                || data.state.settings.connectivity_probe_enabled
                    != input.connectivity_probe_enabled)
        {
            return Err(ApiError::conflict("运行期间只能修改检查周期和总流量"));
        }
        let mut candidate = data.state.settings.clone();
        candidate.interval = input.interval;
        candidate.traffic_total_gb = input.traffic_total_gb.trim().into();
        candidate.portal_auth_enabled = input.portal_auth_enabled;
        candidate.connectivity_probe_enabled = input.connectivity_probe_enabled;
        candidate.validate().map_err(ApiError::bad_request)?;
    }
    state
        .supervisor
        .set_interval(input.interval)
        .await
        .map_err(ApiError::conflict)?;
    let mut data = state.model.lock().unwrap();
    data.state.settings.traffic_total_gb = input.traffic_total_gb.trim().into();
    data.state.settings.portal_auth_enabled = input.portal_auth_enabled;
    data.state.settings.connectivity_probe_enabled = input.connectivity_probe_enabled;
    state
        .storage
        .save_settings(&data.state.settings)
        .map_err(ApiError::internal)?;
    notify(&mut data, &state.events);
    Ok(Json(serde_json::json!({"ok": true})).into_response())
}

#[derive(Deserialize)]
struct InterfacesInput {
    gateway_ip: String,
    interfaces: Vec<String>,
    interface_gateways: HashMap<String, String>,
}

async fn save_interfaces(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(input): ApiJson<InterfacesInput>,
) -> Result<Response, ApiError> {
    authorize_write(&state, &headers)?;
    let mut data = state.model.lock().unwrap();
    if data.state.monitoring || data.state.busy {
        return Err(ApiError::conflict("请先停止当前任务再修改接口"));
    }
    let mut candidate = data.state.settings.clone();
    candidate.gateway_ip = input.gateway_ip.trim().into();
    candidate.interfaces = input.interfaces;
    candidate.interfaces.sort();
    candidate.interfaces.dedup();
    candidate.interface_gateways = input
        .interface_gateways
        .into_iter()
        .filter(|(_, value)| !value.trim().is_empty())
        .map(|(key, value)| (key, value.trim().into()))
        .collect();
    candidate.validate().map_err(ApiError::bad_request)?;
    data.state.settings = candidate;
    state
        .storage
        .save_settings(&data.state.settings)
        .map_err(ApiError::internal)?;
    notify(&mut data, &state.events);
    Ok(Json(serde_json::json!({"ok": true})).into_response())
}

#[derive(Deserialize)]
struct PasswordInput {
    current_password: String,
    new_password: String,
}

async fn change_admin_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(input): ApiJson<PasswordInput>,
) -> Result<Response, ApiError> {
    authorize_write(&state, &headers)?;
    let (session, csrf) = {
        let mut auth = state.auth.lock().unwrap();
        let hash = auth
            .prepare_password_change(&input.current_password, &input.new_password)
            .map_err(ApiError::bad_request)?;
        state
            .storage
            .write_auth_hash(&hash)
            .map_err(ApiError::internal)?;
        auth.commit_password_change(hash)
    };
    Ok(auth_response(session, csrf))
}

async fn action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(action): Path<String>,
) -> Result<Response, ApiError> {
    authorize_write(&state, &headers)?;
    let result = match action.as_str() {
        "start" => state.supervisor.start_monitor().await,
        "stop" => state.supervisor.stop_monitor().await,
        "inspect" => state.supervisor.inspect().await,
        _ => return Err(ApiError::new(StatusCode::NOT_FOUND, "未知操作")),
    };
    result.map_err(ApiError::conflict)?;
    Ok(Json(serde_json::json!({"ok": true})).into_response())
}

async fn test(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(kind): Path<String>,
) -> Result<Response, ApiError> {
    authorize_write(&state, &headers)?;
    let kind = match kind.as_str() {
        "portal" => TestKind::PortalAuth,
        "connectivity" => TestKind::Connectivity,
        _ => return Err(ApiError::new(StatusCode::NOT_FOUND, "未知测试")),
    };
    state
        .supervisor
        .test(kind)
        .await
        .map_err(ApiError::conflict)?;
    Ok(Json(serde_json::json!({"ok": true})).into_response())
}

async fn download_log(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    authenticate(&state, &headers)?;
    let bytes = state.storage.current_log().map_err(ApiError::internal)?;
    Ok((
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=BUAALogin-current.log",
            ),
        ],
        bytes,
    )
        .into_response())
}

struct AuthContext {
    session: String,
    csrf: String,
}

fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<AuthContext, ApiError> {
    let session = cookie(headers, "buaa_session")
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "请先登录"))?;
    let info = state
        .auth
        .lock()
        .unwrap()
        .session(&session)
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "登录已过期"))?;
    Ok(AuthContext {
        session,
        csrf: info.csrf,
    })
}

fn authorize_write(state: &AppState, headers: &HeaderMap) -> Result<AuthContext, ApiError> {
    let context = authenticate(state, headers)?;
    verify_write(headers, &context.csrf)?;
    Ok(context)
}

fn verify_write(headers: &HeaderMap, csrf: &str) -> Result<(), ApiError> {
    verify_origin(headers)?;
    if headers
        .get("x-csrf-token")
        .and_then(|value| value.to_str().ok())
        != Some(csrf)
    {
        return Err(ApiError::bad_request("CSRF 校验失败"));
    }
    Ok(())
}

fn verify_origin(headers: &HeaderMap) -> Result<(), ApiError> {
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::bad_request("缺少 Host"))?;
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::bad_request("缺少 Origin"))?;
    if origin != format!("http://{host}") && origin != format!("https://{host}") {
        return Err(ApiError::bad_request("Origin 校验失败"));
    }
    Ok(())
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(key, value)| (key == name).then(|| value.to_owned()))
}

fn auth_response(session: String, csrf: String) -> Response {
    let mut response = Json(serde_json::json!({"ok": true, "csrf": csrf})).into_response();
    let cookie =
        format!("buaa_session={session}; Path=/; HttpOnly; SameSite=Strict; Max-Age=86400");
    response
        .headers_mut()
        .insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
    response
}

pub struct ApiError {
    status: StatusCode,
    message: String,
}

struct ApiJson<T>(T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(
        request: axum::extract::Request,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        Json::<T>::from_request(request, state)
            .await
            .map(|Json(value)| Self(value))
            .map_err(|error| ApiError::bad_request(error.body_text()))
    }
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, message)
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({"error": self.message})),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::to_bytes, http::Request};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tower::ServiceExt;

    fn test_state() -> (Router, Supervisor, String) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let storage = Storage::new(
            std::env::temp_dir().join(format!("buaalogin-web-api-{}-{unique}", std::process::id())),
        )
        .unwrap();
        let auth_state = AuthState::new(None);
        let setup_token = auth_state.setup_token().unwrap().to_owned();
        let auth = Arc::new(Mutex::new(auth_state));
        let model = Arc::new(Mutex::new(Model::new(
            crate::storage::Settings::default(),
            None,
        )));
        let (events, _) = broadcast::channel(8);
        let supervisor = Supervisor::start(storage.clone(), model.clone(), events.clone());
        (
            router(AppState {
                storage,
                model,
                auth,
                supervisor: supervisor.clone(),
                events,
            }),
            supervisor,
            setup_token,
        )
    }

    fn json_request(method: &str, uri: &str, body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::HOST, "localhost")
            .header(header::ORIGIN, "http://localhost")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn setup_session_and_csrf_contract() {
        let (router, supervisor, setup_token) = test_state();
        let bootstrap = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/bootstrap")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bootstrap.status(), StatusCode::OK);

        let setup = router
            .clone()
            .oneshot(json_request(
                "POST",
                "/api/setup",
                serde_json::json!({"token": setup_token, "password": "password1"}),
            ))
            .await
            .unwrap();
        assert_eq!(setup.status(), StatusCode::OK);
        let cookie = setup.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        let bytes = to_bytes(setup.into_body(), 64 * 1024).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let csrf = payload["csrf"].as_str().unwrap();

        let unauthorized = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/state")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let malformed = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/login")
                    .header(header::HOST, "localhost")
                    .header(header::ORIGIN, "http://localhost")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);

        let mut rejected = json_request(
            "PUT",
            "/api/settings",
            serde_json::json!({
                "interval": 300,
                "traffic_total_gb": "100",
                "portal_auth_enabled": true,
                "connectivity_probe_enabled": true
            }),
        );
        rejected
            .headers_mut()
            .insert(header::COOKIE, cookie.parse().unwrap());
        let rejected = router.clone().oneshot(rejected).await.unwrap();
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);

        let mut accepted = json_request(
            "PUT",
            "/api/settings",
            serde_json::json!({
                "interval": 301,
                "traffic_total_gb": "100",
                "portal_auth_enabled": true,
                "connectivity_probe_enabled": true
            }),
        );
        accepted
            .headers_mut()
            .insert(header::COOKIE, cookie.parse().unwrap());
        accepted
            .headers_mut()
            .insert("x-csrf-token", csrf.parse().unwrap());
        let accepted = router.clone().oneshot(accepted).await.unwrap();
        assert_eq!(accepted.status(), StatusCode::OK);

        let mut interfaces = json_request(
            "PUT",
            "/api/interfaces",
            serde_json::json!({
                "gateway_ip": "10.200.21.4",
                "interfaces": ["wlan0", "eth0", "wlan0"],
                "interface_gateways": {"wlan0": "10.0.0.1"}
            }),
        );
        interfaces
            .headers_mut()
            .insert(header::COOKIE, cookie.parse().unwrap());
        interfaces
            .headers_mut()
            .insert("x-csrf-token", csrf.parse().unwrap());
        assert_eq!(
            router.clone().oneshot(interfaces).await.unwrap().status(),
            StatusCode::OK
        );

        let state = router
            .oneshot(
                Request::builder()
                    .uri("/api/state")
                    .header(header::COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let state = to_bytes(state.into_body(), 64 * 1024).await.unwrap();
        let state: serde_json::Value = serde_json::from_slice(&state).unwrap();
        assert_eq!(
            state["settings"]["interfaces"],
            serde_json::json!(["eth0", "wlan0"])
        );
        supervisor.shutdown().await;
    }
}
