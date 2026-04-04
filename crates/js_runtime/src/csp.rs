// Content Security Policy implementation
use std::collections::HashSet;

/// Content Security Policy for controlling resource loading
#[derive(Debug, Clone)]
pub struct ContentSecurityPolicy {
    /// Allowed origins for scripts (script-src)
    pub script_src: HashSet<String>,
    /// Allowed origins for fetch/XHR (connect-src)
    pub connect_src: HashSet<String>,
    /// Whether to allow 'self' for scripts
    pub allow_self_scripts: bool,
    /// Whether to allow 'self' for fetch
    pub allow_self_connect: bool,
    /// Whether to allow inline scripts (eval, Function)
    pub allow_unsafe_eval: bool,
}

impl ContentSecurityPolicy {
    /// Create a permissive CSP (allows everything)
    pub fn permissive() -> Self {
        Self {
            script_src: HashSet::new(),
            connect_src: HashSet::new(),
            allow_self_scripts: true,
            allow_self_connect: true,
            allow_unsafe_eval: true,
        }
    }

    /// Create a restrictive CSP (only allows specified origins)
    pub fn restrictive() -> Self {
        Self {
            script_src: HashSet::new(),
            connect_src: HashSet::new(),
            allow_self_scripts: false,
            allow_self_connect: false,
            allow_unsafe_eval: false,
        }
    }

    /// Parse CSP from a header value
    /// Example: "script-src 'self' https://example.com; connect-src *"
    pub fn from_header(header: &str) -> Self {
        let mut csp = Self::restrictive();

        for directive in header.split(';') {
            let directive = directive.trim();
            if directive.is_empty() {
                continue;
            }

            let mut parts = directive.split_whitespace();
            if let Some(directive_name) = parts.next() {
                let values: Vec<&str> = parts.collect();

                match directive_name {
                    "script-src" => {
                        for value in values {
                            match value {
                                "'self'" => csp.allow_self_scripts = true,
                                "'unsafe-eval'" => csp.allow_unsafe_eval = true,
                                "*" => {
                                    // Allow all origins
                                    csp.script_src.clear();
                                    csp.allow_self_scripts = true;
                                }
                                origin => {
                                    csp.script_src.insert(origin.to_string());
                                }
                            }
                        }
                    }
                    "connect-src" => {
                        for value in values {
                            match value {
                                "'self'" => csp.allow_self_connect = true,
                                "*" => {
                                    csp.connect_src.clear();
                                    csp.allow_self_connect = true;
                                }
                                origin => {
                                    csp.connect_src.insert(origin.to_string());
                                }
                            }
                        }
                    }
                    _ => {
                        // Ignore unknown directives
                    }
                }
            }
        }

        csp
    }

    /// Check if a script from given URL is allowed
    pub fn allows_script(&self, url: &str, document_origin: &str) -> bool {
        // If script_src is empty and allow_self is true, allow everything from same origin
        if self.script_src.is_empty() && self.allow_self_scripts {
            return self.is_same_origin(url, document_origin);
        }

        // Check if URL matches any allowed origin
        if let Some(origin) = Self::extract_origin(url) {
            if self.allow_self_scripts && self.is_same_origin(url, document_origin) {
                return true;
            }

            for allowed_origin in &self.script_src {
                if origin.starts_with(allowed_origin) || allowed_origin == "*" {
                    return true;
                }
            }
        }

        false
    }

    /// Check if a fetch to given URL is allowed
    pub fn allows_connect(&self, url: &str, document_origin: &str) -> bool {
        if self.connect_src.is_empty() && self.allow_self_connect {
            return self.is_same_origin(url, document_origin);
        }

        if let Some(origin) = Self::extract_origin(url) {
            if self.allow_self_connect && self.is_same_origin(url, document_origin) {
                return true;
            }

            for allowed_origin in &self.connect_src {
                if origin.starts_with(allowed_origin) || allowed_origin == "*" {
                    return true;
                }
            }
        }

        false
    }

    /// Extract origin from URL (protocol + host + port)
    fn extract_origin(url: &str) -> Option<String> {
        // Simple parsing: "http://example.com:8080/path" -> "http://example.com:8080"
        if let Some(after_protocol) = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://")) {
            let protocol = if url.starts_with("https://") { "https://" } else { "http://" };

            // Find end of host (before path)
            let host_end = after_protocol.find('/').unwrap_or(after_protocol.len());
            let host = &after_protocol[..host_end];

            Some(format!("{}{}", protocol, host))
        } else {
            None
        }
    }

    /// Check if two URLs have the same origin
    fn is_same_origin(&self, url1: &str, url2: &str) -> bool {
        match (Self::extract_origin(url1), Self::extract_origin(url2)) {
            (Some(origin1), Some(origin2)) => origin1 == origin2,
            _ => false,
        }
    }

    /// Add an allowed script origin
    pub fn add_script_src(&mut self, origin: String) {
        self.script_src.insert(origin);
    }

    /// Add an allowed connect origin
    pub fn add_connect_src(&mut self, origin: String) {
        self.connect_src.insert(origin);
    }
}

/// CORS validation helper
pub struct CorsValidator;

impl CorsValidator {
    /// Check if a fetch request should be allowed based on CORS
    /// For now, this is a simplified implementation
    pub fn validate_request(request_url: &str, document_origin: &str) -> CorsValidation {
        // Same-origin requests always allowed
        if Self::is_same_origin(request_url, document_origin) {
            return CorsValidation::Allowed;
        }

        // Cross-origin requests require CORS headers (validated after response)
        CorsValidation::RequiresCors
    }

    /// Validate CORS response headers
    pub fn validate_response(
        request_origin: &str,
        access_control_allow_origin: Option<&str>,
    ) -> bool {
        if let Some(allowed_origin) = access_control_allow_origin {
            allowed_origin == "*" || allowed_origin == request_origin
        } else {
            false
        }
    }

    fn is_same_origin(url1: &str, url2: &str) -> bool {
        ContentSecurityPolicy::extract_origin(url1) == ContentSecurityPolicy::extract_origin(url2)
    }
}

#[derive(Debug, PartialEq)]
pub enum CorsValidation {
    Allowed,
    RequiresCors,
    Blocked,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_csp_parsing() {
        let csp = ContentSecurityPolicy::from_header("script-src 'self' https://cdn.example.com");
        assert!(csp.allow_self_scripts);
        assert!(csp.script_src.contains("https://cdn.example.com"));
    }

    #[test]
    fn test_csp_allows_script() {
        let mut csp = ContentSecurityPolicy::restrictive();
        csp.add_script_src("https://cdn.example.com".to_string());
        csp.allow_self_scripts = true;

        assert!(csp.allows_script("https://example.com/script.js", "https://example.com"));
        assert!(csp.allows_script("https://cdn.example.com/lib.js", "https://example.com"));
        assert!(!csp.allows_script("https://evil.com/malware.js", "https://example.com"));
    }

    #[test]
    fn test_origin_extraction() {
        assert_eq!(
            ContentSecurityPolicy::extract_origin("https://example.com/path"),
            Some("https://example.com".to_string())
        );
        assert_eq!(
            ContentSecurityPolicy::extract_origin("http://localhost:8080/api"),
            Some("http://localhost:8080".to_string())
        );
    }

    #[test]
    fn test_cors_validation() {
        assert_eq!(
            CorsValidator::validate_request("https://example.com/api", "https://example.com"),
            CorsValidation::Allowed
        );
        assert_eq!(
            CorsValidator::validate_request("https://api.other.com/data", "https://example.com"),
            CorsValidation::RequiresCors
        );
    }
}
