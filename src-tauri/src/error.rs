use serde::Serialize;
use zip::result::ZipError;

#[derive(Debug, Clone, Serialize)]
pub struct CyberAPIError {
    message: String,
    category: String,
}

impl From<sea_orm::DbErr> for CyberAPIError {
    fn from(error: sea_orm::DbErr) -> Self {
        CyberAPIError {
            message: error.to_string(),
            category: "seaOrm".to_string(),
        }
    }
}
impl From<hyper::http::Error> for CyberAPIError {
    fn from(error: hyper::http::Error) -> Self {
        CyberAPIError {
            message: error.to_string(),
            category: "http".to_string(),
        }
    }
}
impl From<hyper::Error> for CyberAPIError {
    fn from(error: hyper::Error) -> Self {
        CyberAPIError {
            message: error.to_string(),
            category: "http".to_string(),
        }
    }
}
impl From<tauri::http::InvalidUri> for CyberAPIError {
    fn from(error: tauri::http::InvalidUri) -> Self {
        CyberAPIError {
            message: error.to_string(),
            category: "invalidUri".to_string(),
        }
    }
}
impl From<hyper::header::InvalidHeaderValue> for CyberAPIError {
    fn from(error: hyper::header::InvalidHeaderValue) -> Self {
        CyberAPIError {
            message: error.to_string(),
            category: "invalidHeader".to_string(),
        }
    }
}

impl From<hyper::header::InvalidHeaderName> for CyberAPIError {
    fn from(error: hyper::header::InvalidHeaderName) -> Self {
        CyberAPIError {
            message: error.to_string(),
            category: "invalidHeaderName".to_string(),
        }
    }
}

impl From<hyper::header::ToStrError> for CyberAPIError {
    fn from(error: hyper::header::ToStrError) -> Self {
        CyberAPIError {
            message: error.to_string(),
            category: "toStrError".to_string(),
        }
    }
}

impl From<std::io::Error> for CyberAPIError {
    fn from(error: std::io::Error) -> Self {
        CyberAPIError {
            message: error.to_string(),
            category: "io".to_string(),
        }
    }
}
impl From<cookie_store::Error> for CyberAPIError {
    fn from(error: cookie_store::Error) -> Self {
        CyberAPIError {
            message: error.to_string(),
            category: "cookieStore".to_string(),
        }
    }
}

impl From<url::ParseError> for CyberAPIError {
    fn from(error: url::ParseError) -> Self {
        CyberAPIError {
            message: error.to_string(),
            category: "urlParse".to_string(),
        }
    }
}

impl From<cookie_store::CookieError> for CyberAPIError {
    fn from(error: cookie_store::CookieError) -> Self {
        CyberAPIError {
            message: error.to_string(),
            category: "cookieStore".to_string(),
        }
    }
}

impl From<serde_json::Error> for CyberAPIError {
    fn from(error: serde_json::Error) -> Self {
        CyberAPIError {
            message: error.to_string(),
            category: "serdeJson".to_string(),
        }
    }
}

impl From<base64::DecodeError> for CyberAPIError {
    fn from(error: base64::DecodeError) -> Self {
        CyberAPIError {
            message: error.to_string(),
            category: "base64".to_string(),
        }
    }
}

impl From<ZipError> for CyberAPIError {
    fn from(error: ZipError) -> Self {
        CyberAPIError {
            message: error.to_string(),
            category: "zip".to_string(),
        }
    }
}
impl From<cookie::ParseError> for CyberAPIError {
    fn from(error: cookie::ParseError) -> Self {
        CyberAPIError {
            message: error.to_string(),
            category: "cookie".to_string(),
        }
    }
}
