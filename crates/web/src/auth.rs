use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

const SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const FAILURE_WINDOW: Duration = Duration::from_secs(60);
const BLOCK_TIME: Duration = Duration::from_secs(5 * 60);

#[derive(Clone)]
pub struct SessionInfo {
    pub csrf: String,
    pub expires: Instant,
}

pub struct AuthState {
    password_hash: Option<String>,
    setup_token: Option<String>,
    sessions: HashMap<String, SessionInfo>,
    failures: VecDeque<Instant>,
    blocked_until: Option<Instant>,
}

impl AuthState {
    pub fn new(password_hash: Option<String>) -> Self {
        Self {
            setup_token: password_hash.is_none().then(random_token),
            password_hash,
            sessions: HashMap::new(),
            failures: VecDeque::new(),
            blocked_until: None,
        }
    }

    pub fn needs_setup(&self) -> bool {
        self.password_hash.is_none()
    }

    pub fn setup_token(&self) -> Option<&str> {
        self.setup_token.as_deref()
    }

    pub fn prepare_setup(&self, token: &str, password: &str) -> Result<String, String> {
        if !self.needs_setup() {
            return Err("网页管理密码已经设置".into());
        }
        if !constant_time_eq(
            self.setup_token.as_deref().unwrap_or_default(),
            token.trim(),
        ) {
            return Err("一次性设置码无效".into());
        }
        validate_password(password)?;
        hash_password(password)
    }

    pub fn commit_setup(&mut self, hash: String) -> Result<(String, String), String> {
        if !self.needs_setup() {
            return Err("网页管理密码已经设置".into());
        }
        self.password_hash = Some(hash);
        self.setup_token = None;
        Ok(self.create_session())
    }

    pub fn login(&mut self, password: &str) -> Result<(String, String), LoginError> {
        let now = Instant::now();
        if self.blocked_until.is_some_and(|until| until > now) {
            return Err(LoginError::Limited);
        }
        while self
            .failures
            .front()
            .is_some_and(|attempt| now.duration_since(*attempt) > FAILURE_WINDOW)
        {
            self.failures.pop_front();
        }
        let valid = self
            .password_hash
            .as_deref()
            .is_some_and(|hash| verify_password(hash, password));
        if !valid {
            self.failures.push_back(now);
            if self.failures.len() >= 5 {
                self.blocked_until = Some(now + BLOCK_TIME);
                self.failures.clear();
                return Err(LoginError::Limited);
            }
            return Err(LoginError::Invalid);
        }
        self.failures.clear();
        self.blocked_until = None;
        Ok(self.create_session())
    }

    pub fn session(&mut self, id: &str) -> Option<SessionInfo> {
        let now = Instant::now();
        self.sessions.retain(|_, session| session.expires > now);
        self.sessions.get(id).cloned()
    }

    pub fn logout(&mut self, id: &str) {
        self.sessions.remove(id);
    }

    pub fn prepare_password_change(
        &self,
        current: &str,
        new_password: &str,
    ) -> Result<String, String> {
        let valid = self
            .password_hash
            .as_deref()
            .is_some_and(|hash| verify_password(hash, current));
        if !valid {
            return Err("当前管理密码错误".into());
        }
        validate_password(new_password)?;
        hash_password(new_password)
    }

    pub fn commit_password_change(&mut self, hash: String) -> (String, String) {
        self.password_hash = Some(hash);
        self.sessions.clear();
        self.create_session()
    }

    fn create_session(&mut self) -> (String, String) {
        let id = random_token();
        let csrf = random_token();
        self.sessions.insert(
            id.clone(),
            SessionInfo {
                csrf: csrf.clone(),
                expires: Instant::now() + SESSION_TTL,
            },
        );
        (id, csrf)
    }
}

pub enum LoginError {
    Invalid,
    Limited,
}

pub fn validate_password(password: &str) -> Result<(), String> {
    if password.chars().count() < 8 {
        return Err("管理密码至少需要 8 个字符".into());
    }
    Ok(())
}

fn hash_password(password: &str) -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| "无法生成密码盐值")?;
    let salt = SaltString::encode_b64(&bytes).map_err(|_| "无法生成密码盐值")?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| "无法保存管理密码".into())
}

fn verify_password(hash: &str, password: &str) -> bool {
    PasswordHash::new(hash).ok().is_some_and(|hash| {
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    })
}

pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("operating-system randomness is required");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.bytes()
        .zip(right.bytes())
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_login_and_session_expiry_contract() {
        let mut auth = AuthState::new(None);
        let token = auth.setup_token().unwrap().to_owned();
        assert!(auth.prepare_setup("wrong", "password1").is_err());
        let hash = auth.prepare_setup(&token, "password1").unwrap();
        let (session, csrf) = auth.commit_setup(hash).unwrap();
        assert_eq!(auth.session(&session).unwrap().csrf, csrf);
        assert!(auth.login("bad").is_err());
        assert!(auth.login("password1").is_ok());
    }

    #[test]
    fn limits_repeated_login_failures() {
        let mut auth = AuthState::new(None);
        let token = auth.setup_token().unwrap().to_owned();
        let hash = auth.prepare_setup(&token, "password1").unwrap();
        auth.commit_setup(hash).unwrap();
        for _ in 0..4 {
            assert!(matches!(auth.login("wrong"), Err(LoginError::Invalid)));
        }
        assert!(matches!(auth.login("wrong"), Err(LoginError::Limited)));
        assert!(matches!(auth.login("password1"), Err(LoginError::Limited)));
    }
}
