//! 호스트 공통 config 조각 (generic).
//!
//! 서비스별 config(MC 의 RCON·로그 경로 등)는 각 flavor 바이너리가 보유.
//! core 는 시크릿 마스킹 타입만 제공.

/// Secret string — Debug/Display 에서 마스킹.
#[derive(Clone)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(s: String) -> Self {
        Self(s)
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("\"***\"")
    }
}

impl std::fmt::Display for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("***")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_masks() {
        let s = SecretString::new("super-secret".into());
        assert_eq!(format!("{:?}", s), "\"***\"");
        assert_eq!(s.expose(), "super-secret");
    }
}
