//! Stable error envelope shared by Rust commands and the web UI.
//!
//! Human-readable details are intentionally not the contract: providers may return them in
//! different languages, and our own diagnostics can be reworded. The frontend classifies by
//! `code` and only shows `detail` as secondary diagnostic text.

const PREFIX: &str = "sayit_error:";

pub fn encode(code: &str, detail: impl AsRef<str>) -> String {
    format!("{}{}:{}", PREFIX, code, detail.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_colons_and_unicode_in_detail() {
        assert_eq!(
            encode("provider_bad_key", "HTTP 401: 密钥无效"),
            "sayit_error:provider_bad_key:HTTP 401: 密钥无效"
        );
    }
}
