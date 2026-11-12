//! Intel Snow Leopard OpenGL compatibility helpers. Used only by the opt-in
//! legacy macOS build; tests are platform-independent.

pub(crate) fn symbol_alias(name: &[u8]) -> Option<&'static [u8]> {
    Some(match name {
        b"glGenVertexArrays" => b"glGenVertexArraysAPPLE\0",
        b"glBindVertexArray" => b"glBindVertexArrayAPPLE\0",
        b"glDeleteVertexArrays" => b"glDeleteVertexArraysAPPLE\0",
        b"glGenFramebuffers" => b"glGenFramebuffersEXT\0",
        b"glBindFramebuffer" => b"glBindFramebufferEXT\0",
        b"glDeleteFramebuffers" => b"glDeleteFramebuffersEXT\0",
        b"glCheckFramebufferStatus" => b"glCheckFramebufferStatusEXT\0",
        b"glFramebufferTexture2D" => b"glFramebufferTexture2DEXT\0",
        b"glFramebufferRenderbuffer" => b"glFramebufferRenderbufferEXT\0",
        b"glGenRenderbuffers" => b"glGenRenderbuffersEXT\0",
        b"glBindRenderbuffer" => b"glBindRenderbufferEXT\0",
        b"glDeleteRenderbuffers" => b"glDeleteRenderbuffersEXT\0",
        b"glRenderbufferStorage" => b"glRenderbufferStorageEXT\0",
        b"glGenerateMipmap" => b"glGenerateMipmapEXT\0",
        // present on OS X 10.6's OpenGL 2.1 under these names only (asked of a Mac Pro 3,1's OpenGL framework)
        b"glDrawArraysInstanced" => b"glDrawArraysInstancedARB\0",
        b"glDrawElementsInstanced" => b"glDrawElementsInstancedARB\0",
        b"glVertexAttribDivisor" => b"glVertexAttribDivisorARB\0",
        b"glVertexAttribIPointer" => b"glVertexAttribIPointerEXT\0",
        _ => return None,
    })
}

/// Converts the GLES 1.00 shaders supplied by macroquad and the game to desktop
/// GLSL 1.20. Only full precision-keyword tokens are removed, never substrings
/// of identifiers. Other GLSL versions are left untouched.
pub(crate) fn desktop_shader(source: &str) -> String {
    if !source.lines().any(|line| line.trim() == "#version 100") {
        return source.to_owned();
    }
    let mut out = String::with_capacity(source.len());
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed == "#version 100" {
            out.push_str("#version 120\n");
            continue;
        }
        if trimmed.starts_with("precision ") && trimmed.ends_with(';') {
            out.push('\n');
            continue;
        }
        let mut token = String::new();
        for ch in line.chars().chain(std::iter::once('\n')) {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                token.push(ch);
            } else {
                if !matches!(token.as_str(), "lowp" | "mediump" | "highp") {
                    out.push_str(&token);
                }
                token.clear();
                out.push(ch);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_legacy_extensions_without_affecting_normal_symbols() {
        assert_eq!(symbol_alias(b"glBindVertexArray"), Some(&b"glBindVertexArrayAPPLE\0"[..]));
        assert_eq!(symbol_alias(b"glBindFramebuffer"), Some(&b"glBindFramebufferEXT\0"[..]));
        assert_eq!(symbol_alias(b"glCreateShader"), None);
        assert_eq!(symbol_alias(b"glDrawElements"), None);
    }

    #[test]
    fn translates_precision_but_preserves_identifier_names() {
        let converted = desktop_shader("#version 100\nprecision highp float;\nvarying highp vec2 uv;\nfloat highpass = 1.0;\n");
        assert!(converted.starts_with("#version 120\n"));
        assert!(!converted.contains("precision"));
        assert!(converted.contains("varying  vec2 uv;"));
        assert!(converted.contains("highpass = 1.0"));
    }

    #[test]
    fn leaves_other_shader_versions_byte_identical() {
        for source in ["#version 150\nin vec2 uv;", "#version 330\nvoid main() {}"] {
            assert_eq!(desktop_shader(source), source);
        }
    }
}
