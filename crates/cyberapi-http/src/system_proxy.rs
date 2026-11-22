//! R2: reads the desktop OS system proxy and evaluates PAC scripts using OS
//! primitives (macOS SystemConfiguration + CFNetwork, Windows WinHTTP) without
//! embedding any JavaScript engine. Linux/other platforms return `None`.

use crate::native::Desktop;
use cyberapi_core::proxy::Proxy;
use url::Url;

pub(crate) async fn resolve(url: &Url, desktop: &Desktop) -> Result<Option<Proxy>, String> {
    #[cfg(target_os = "macos")]
    {
        macos::resolve(url, desktop.certificates).await
    }
    #[cfg(target_os = "windows")]
    {
        let _ = desktop;
        windows::resolve(url).await
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (url, desktop);
        Ok(None)
    }
}

#[cfg(all(test, target_os = "macos"))]
pub(crate) async fn eval_pac_script_for_test(script: &str, url: &Url) -> Result<Option<Proxy>, String> {
    macos::eval_pac_script(script.to_string(), url.as_str().to_string()).await
}

#[cfg(all(test, target_os = "macos"))]
pub(crate) async fn fetch_and_eval_pac_for_test(
    pac_url: &str,
    target_url: &Url,
    certificates: bool,
) -> Result<Option<Proxy>, String> {
    let script = macos::fetch_pac_script(pac_url, certificates).await?;
    macos::eval_pac_script(script, target_url.as_str().to_string()).await
}

#[cfg(target_os = "macos")]
mod macos {
    use crate::TargetPolicy;
    use crate::native::{Desktop, Reading};
    use core_foundation_sys::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
    use core_foundation_sys::base::{CFEqual, CFGetTypeID, CFRelease, CFTypeRef, kCFAllocatorDefault};
    use core_foundation_sys::dictionary::{CFDictionaryGetTypeID, CFDictionaryGetValue, CFDictionaryRef};
    use core_foundation_sys::error::{CFErrorCopyDescription, CFErrorRef};
    use core_foundation_sys::number::{CFNumberGetTypeID, CFNumberGetValue, CFNumberRef, kCFNumberSInt32Type};
    use core_foundation_sys::string::{
        CFStringCreateWithBytes, CFStringGetCString, CFStringGetLength, CFStringGetMaximumSizeForEncoding,
        CFStringGetTypeID, CFStringRef, kCFStringEncodingUTF8,
    };
    use core_foundation_sys::url::{CFURLCreateWithBytes, CFURLGetString, CFURLGetTypeID, CFURLRef};
    use cyberapi_core::model::HttpMethod;
    use cyberapi_core::prefs::Timeouts;
    use cyberapi_core::proxy::{Proxy, ProxyKind, ProxyMode, ProxyPrefs};
    use cyberapi_core::send::PreparedRequest;
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::os::raw::c_char;
    use std::ptr;
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};
    use url::Url;

    const PAC_CACHE_TTL: Duration = Duration::from_secs(60);
    const PAC_MAX_BYTES: usize = 1024 * 1024;
    const PAC_MAX_REDIRECTS: usize = 3;

    #[link(name = "SystemConfiguration", kind = "framework")]
    unsafe extern "C" {
        fn SCDynamicStoreCopyProxies(store: *const c_void) -> CFDictionaryRef;
    }

    #[link(name = "CFNetwork", kind = "framework")]
    unsafe extern "C" {
        static kCFProxyTypeKey: CFStringRef;
        static kCFProxyHostNameKey: CFStringRef;
        static kCFProxyPortNumberKey: CFStringRef;
        static kCFProxyUsernameKey: CFStringRef;
        static kCFProxyPasswordKey: CFStringRef;
        static kCFProxyAutoConfigurationURLKey: CFStringRef;
        static kCFProxyAutoConfigurationJavaScriptKey: CFStringRef;

        static kCFProxyTypeNone: CFStringRef;
        static kCFProxyTypeHTTP: CFStringRef;
        static kCFProxyTypeHTTPS: CFStringRef;
        static kCFProxyTypeSOCKS: CFStringRef;
        static kCFProxyTypeAutoConfigurationURL: CFStringRef;
        static kCFProxyTypeAutoConfigurationJavaScript: CFStringRef;

        fn CFNetworkCopyProxiesForURL(url: CFURLRef, proxy_settings: CFDictionaryRef) -> CFArrayRef;
        fn CFNetworkCopyProxiesForAutoConfigurationScript(
            proxy_auto_configuration_script: CFStringRef,
            target_url: CFURLRef,
            error: *mut CFErrorRef,
        ) -> CFArrayRef;
    }

    enum MacCandidate {
        Direct,
        Proxy(Proxy),
        PacScript(String),
        PacUrl(String),
    }

    struct CfOwned {
        ptr: CFTypeRef,
    }

    impl CfOwned {
        fn from_raw<T>(ptr: *const T) -> Option<Self> {
            (!ptr.is_null()).then_some(Self {
                ptr: ptr.cast::<c_void>(),
            })
        }

        fn as_ptr<T>(&self) -> *const T {
            self.ptr.cast::<T>()
        }
    }

    impl Drop for CfOwned {
        fn drop(&mut self) {
            if !self.ptr.is_null() {
                unsafe {
                    CFRelease(self.ptr);
                }
            }
        }
    }

    fn pac_cache() -> &'static Mutex<HashMap<String, (Instant, String)>> {
        static CACHE: OnceLock<Mutex<HashMap<String, (Instant, String)>>> = OnceLock::new();
        CACHE.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn cf_string_new(text: &str) -> Option<CfOwned> {
        let bytes = text.as_bytes();
        let ptr = unsafe {
            CFStringCreateWithBytes(
                kCFAllocatorDefault,
                bytes.as_ptr(),
                bytes.len() as isize,
                kCFStringEncodingUTF8,
                0,
            )
        };
        CfOwned::from_raw(ptr)
    }

    fn cf_url_new(text: &str) -> Option<CfOwned> {
        let bytes = text.as_bytes();
        let ptr = unsafe {
            CFURLCreateWithBytes(
                kCFAllocatorDefault,
                bytes.as_ptr(),
                bytes.len() as isize,
                kCFStringEncodingUTF8,
                ptr::null(),
            )
        };
        CfOwned::from_raw(ptr)
    }

    unsafe fn read_cf_string(raw: CFTypeRef) -> Option<String> {
        if raw.is_null() || unsafe { CFGetTypeID(raw) != CFStringGetTypeID() } {
            return None;
        }
        let s = raw.cast();
        let utf16_len = unsafe { CFStringGetLength(s) };
        let max_bytes = unsafe { CFStringGetMaximumSizeForEncoding(utf16_len, kCFStringEncodingUTF8) };
        if max_bytes < 0 {
            return None;
        }
        let mut buf = vec![0u8; max_bytes as usize + 1];
        let ok = unsafe {
            CFStringGetCString(
                s,
                buf.as_mut_ptr().cast::<c_char>(),
                buf.len() as isize,
                kCFStringEncodingUTF8,
            )
        };
        if ok == 0 {
            return None;
        }
        let nul = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
        String::from_utf8(buf[..nul].to_vec()).ok()
    }

    unsafe fn read_cf_url_string(raw: CFTypeRef) -> Option<String> {
        if raw.is_null() {
            return None;
        }
        if unsafe { CFGetTypeID(raw) == CFStringGetTypeID() } {
            return unsafe { read_cf_string(raw) };
        }
        if unsafe { CFGetTypeID(raw) == CFURLGetTypeID() } {
            let cf_str = unsafe { CFURLGetString(raw.cast()) };
            return unsafe { read_cf_string(cf_str.cast()) };
        }
        None
    }

    unsafe fn read_cf_port(raw: CFTypeRef) -> Option<u16> {
        if raw.is_null() || unsafe { CFGetTypeID(raw) != CFNumberGetTypeID() } {
            return None;
        }
        let mut val: i32 = 0;
        let ok = unsafe { CFNumberGetValue(raw as CFNumberRef, kCFNumberSInt32Type, (&raw mut val).cast::<c_void>()) };
        (ok && (0..=65535).contains(&val)).then_some(val as u16)
    }

    unsafe fn cf_strings_equal(a: CFTypeRef, b: CFStringRef) -> bool {
        !a.is_null() && !b.is_null() && unsafe { CFEqual(a, b.cast()) != 0 }
    }

    unsafe fn read_proxy_array(arr: CFArrayRef) -> Vec<MacCandidate> {
        let mut out = Vec::new();
        if arr.is_null() {
            return out;
        }
        let count = unsafe { CFArrayGetCount(arr) };
        for i in 0..count {
            let item = unsafe { CFArrayGetValueAtIndex(arr, i) };
            if item.is_null() || unsafe { CFGetTypeID(item) != CFDictionaryGetTypeID() } {
                continue;
            }
            let dict: CFDictionaryRef = item.cast();
            let kind_val = unsafe { CFDictionaryGetValue(dict, kCFProxyTypeKey.cast()) };
            if unsafe { cf_strings_equal(kind_val, kCFProxyTypeNone) } {
                out.push(MacCandidate::Direct);
                continue;
            }

            let proxy_kind = if unsafe {
                cf_strings_equal(kind_val, kCFProxyTypeHTTP) || cf_strings_equal(kind_val, kCFProxyTypeHTTPS)
            } {
                Some(ProxyKind::Http)
            } else if unsafe { cf_strings_equal(kind_val, kCFProxyTypeSOCKS) } {
                Some(ProxyKind::Socks5 { remote_dns: true })
            } else {
                None
            };

            if let Some(kind) = proxy_kind {
                let host_raw = unsafe { CFDictionaryGetValue(dict, kCFProxyHostNameKey.cast()) };
                let port_raw = unsafe { CFDictionaryGetValue(dict, kCFProxyPortNumberKey.cast()) };
                let Some(host) = (unsafe { read_cf_string(host_raw) }) else {
                    continue;
                };
                let host = host.trim().trim_start_matches('[').trim_end_matches(']').to_string();
                if host.is_empty() {
                    continue;
                }
                let port = unsafe { read_cf_port(port_raw) }.unwrap_or(1080);
                let user = unsafe { read_cf_string(CFDictionaryGetValue(dict, kCFProxyUsernameKey.cast())) }
                    .filter(|s| !s.is_empty());
                let pass = unsafe { read_cf_string(CFDictionaryGetValue(dict, kCFProxyPasswordKey.cast())) }
                    .unwrap_or_default();
                let auth = user.map(|u| (u, pass));
                out.push(MacCandidate::Proxy(Proxy { kind, host, port, auth }));
                continue;
            }

            if unsafe { cf_strings_equal(kind_val, kCFProxyTypeAutoConfigurationJavaScript) }
                && let Some(script) = unsafe {
                    read_cf_string(CFDictionaryGetValue(
                        dict,
                        kCFProxyAutoConfigurationJavaScriptKey.cast(),
                    ))
                }
                && !script.trim().is_empty()
            {
                out.push(MacCandidate::PacScript(script));
                continue;
            }

            if unsafe { cf_strings_equal(kind_val, kCFProxyTypeAutoConfigurationURL) }
                && let Some(pac_url) =
                    unsafe { read_cf_url_string(CFDictionaryGetValue(dict, kCFProxyAutoConfigurationURLKey.cast())) }
                && !pac_url.trim().is_empty()
            {
                out.push(MacCandidate::PacUrl(pac_url.trim().to_string()));
            }
        }
        out
    }

    fn system_candidates(url_str: &str) -> Vec<MacCandidate> {
        let Some(settings) = CfOwned::from_raw(unsafe { SCDynamicStoreCopyProxies(ptr::null()) }) else {
            return Vec::new();
        };
        let Some(cf_url) = cf_url_new(url_str) else {
            return Vec::new();
        };
        let Some(arr) = CfOwned::from_raw(unsafe { CFNetworkCopyProxiesForURL(cf_url.as_ptr(), settings.as_ptr()) })
        else {
            return Vec::new();
        };
        unsafe { read_proxy_array(arr.as_ptr()) }
    }

    pub(super) async fn eval_pac_script(script: String, url_str: String) -> Result<Option<Proxy>, String> {
        let candidates = tokio::task::spawn_blocking(move || -> Result<Vec<MacCandidate>, String> {
            let cf_script = cf_string_new(&script).ok_or_else(|| "invalid PAC script UTF-8".to_string())?;
            let cf_url = cf_url_new(&url_str).ok_or_else(|| "invalid target URL for PAC".to_string())?;
            let mut cf_err: CFErrorRef = ptr::null_mut();
            let raw_arr = unsafe {
                CFNetworkCopyProxiesForAutoConfigurationScript(cf_script.as_ptr(), cf_url.as_ptr(), &raw mut cf_err)
            };
            let _owned_err = CfOwned::from_raw(cf_err);
            let Some(arr) = CfOwned::from_raw(raw_arr) else {
                let msg = if cf_err.is_null() {
                    "PAC script returned no proxies".to_string()
                } else {
                    let desc = CfOwned::from_raw(unsafe { CFErrorCopyDescription(cf_err) });
                    desc.and_then(|d| unsafe { read_cf_string(d.ptr) })
                        .unwrap_or_else(|| "PAC evaluation failed".to_string())
                };
                return Err(msg);
            };
            Ok(unsafe { read_proxy_array(arr.as_ptr()) })
        })
        .await
        .map_err(|err| format!("PAC evaluation task failed: {err}"))??;

        for candidate in candidates {
            match candidate {
                MacCandidate::Direct => return Ok(None),
                MacCandidate::Proxy(proxy) => return Ok(Some(proxy)),
                MacCandidate::PacScript(_) | MacCandidate::PacUrl(_) => continue,
            }
        }
        Ok(None)
    }

    pub(super) async fn fetch_pac_script(pac_url: &str, certificates: bool) -> Result<String, String> {
        if let Ok(mut guard) = pac_cache().lock()
            && let Some((fetched_at, cached)) = guard.get(pac_url)
        {
            if fetched_at.elapsed() < PAC_CACHE_TTL {
                return Ok(cached.clone());
            }
            guard.remove(pac_url);
        }

        let script = if pac_url.to_ascii_lowercase().starts_with("file://") {
            let parsed = Url::parse(pac_url).map_err(|err| format!("invalid PAC file URL {pac_url}: {err}"))?;
            let path = parsed
                .to_file_path()
                .map_err(|()| format!("invalid PAC file path {pac_url}"))?;
            tokio::task::spawn_blocking(move || -> Result<String, String> {
                let meta = std::fs::metadata(&path).map_err(|err| format!("cannot read PAC file: {err}"))?;
                if meta.len() as usize > PAC_MAX_BYTES {
                    return Err("PAC file exceeds 1 MiB limit".into());
                }
                std::fs::read_to_string(&path).map_err(|err| format!("cannot read PAC file: {err}"))
            })
            .await
            .map_err(|err| err.to_string())??
        } else {
            fetch_http_pac_script(pac_url, certificates).await?
        };

        if let Ok(mut guard) = pac_cache().lock() {
            guard.insert(pac_url.to_string(), (Instant::now(), script.clone()));
        }
        Ok(script)
    }

    async fn fetch_http_pac_script(pac_url: &str, certificates: bool) -> Result<String, String> {
        let mut current = pac_url.to_string();
        for _ in 0..=PAC_MAX_REDIRECTS {
            let request = PreparedRequest {
                method: HttpMethod::Get,
                url: current.clone(),
                timeouts: Timeouts {
                    connect: 5,
                    write: 5,
                    read: 5,
                },
                accept_invalid_certs: false,
                proxy: ProxyPrefs {
                    mode: ProxyMode::Off,
                    ..Default::default()
                },
                ..Default::default()
            };
            let exchange = Box::pin(crate::native::exchange(
                request,
                TargetPolicy::Anywhere,
                Desktop {
                    proxies: false,
                    certificates,
                },
                Reading {
                    limit: PAC_MAX_BYTES,
                    watch: None,
                },
            ))
            .await
            .map_err(|err| err.to_string())?;
            let status = exchange.status;
            if matches!(status, 301 | 302 | 303 | 307 | 308)
                && let Some(next_url) = cyberapi_core::redirect::target(&current, status, &exchange.headers)
            {
                current = next_url.to_string();
                continue;
            }
            if !(200..300).contains(&status) {
                return Err(format!("PAC server returned HTTP {status}"));
            }
            if exchange.truncated {
                return Err("PAC script exceeds 1 MiB limit".into());
            }
            return String::from_utf8(exchange.body).map_err(|err| format!("PAC script is not valid UTF-8: {err}"));
        }
        Err("too many redirects fetching PAC script".into())
    }

    pub(super) async fn resolve(url: &Url, certificates: bool) -> Result<Option<Proxy>, String> {
        let url_str = url.as_str().to_string();
        let candidates = system_candidates(&url_str);
        for candidate in candidates {
            match candidate {
                MacCandidate::Direct => return Ok(None),
                MacCandidate::Proxy(proxy) => return Ok(Some(proxy)),
                MacCandidate::PacScript(script) => match eval_pac_script(script, url_str.clone()).await {
                    Ok(Some(proxy)) => return Ok(Some(proxy)),
                    Ok(None) => return Ok(None),
                    Err(why) => {
                        tracing::warn!("macOS PAC script evaluation failed: {why}");
                    }
                },
                MacCandidate::PacUrl(pac_url) => match fetch_pac_script(&pac_url, certificates).await {
                    Ok(script) => match eval_pac_script(script, url_str.clone()).await {
                        Ok(Some(proxy)) => return Ok(Some(proxy)),
                        Ok(None) => return Ok(None),
                        Err(why) => {
                            tracing::warn!("macOS PAC URL {pac_url} evaluation failed: {why}");
                        }
                    },
                    Err(why) => {
                        tracing::warn!("failed to fetch PAC script from {pac_url}: {why}");
                    }
                },
            }
        }
        Ok(None)
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use cyberapi_core::proxy::{Proxy, bypassed_windows, parse_winhttp_proxy_list};
    use std::ffi::c_void;
    use std::ptr;
    use url::Url;
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::Networking::WinHttp::{
        WINHTTP_ACCESS_TYPE_NO_PROXY, WINHTTP_AUTO_DETECT_TYPE_DHCP, WINHTTP_AUTO_DETECT_TYPE_DNS_A,
        WINHTTP_AUTOPROXY_AUTO_DETECT, WINHTTP_AUTOPROXY_CONFIG_URL, WINHTTP_AUTOPROXY_OPTIONS,
        WINHTTP_CURRENT_USER_IE_PROXY_CONFIG, WINHTTP_PROXY_INFO, WinHttpCloseHandle,
        WinHttpGetIEProxyConfigForCurrentUser, WinHttpGetProxyForUrl, WinHttpOpen,
    };

    struct GlobalWide(windows_sys::core::PWSTR);

    impl GlobalWide {
        unsafe fn to_string_opt(&self) -> Option<String> {
            if self.0.is_null() {
                return None;
            }
            let mut len = 0usize;
            while unsafe { *self.0.add(len) } != 0 {
                len += 1;
            }
            let slice = unsafe { std::slice::from_raw_parts(self.0, len) };
            Some(String::from_utf16_lossy(slice))
        }
    }

    impl Drop for GlobalWide {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    GlobalFree(self.0.cast::<c_void>());
                }
            }
        }
    }

    struct WinHttpHandle(*mut c_void);

    impl Drop for WinHttpHandle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    WinHttpCloseHandle(self.0);
                }
            }
        }
    }

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub(super) async fn resolve(url: &Url) -> Result<Option<Proxy>, String> {
        let url_str = url.as_str().to_string();
        let host = url.host_str().unwrap_or_default().to_string();
        let scheme = url.scheme().to_string();
        tokio::task::spawn_blocking(move || {
            let mut ie_cfg = WINHTTP_CURRENT_USER_IE_PROXY_CONFIG::default();
            if unsafe { WinHttpGetIEProxyConfigForCurrentUser(&raw mut ie_cfg) } == 0 {
                return Ok(None);
            }
            let auto_url_guard = GlobalWide(ie_cfg.lpszAutoConfigUrl);
            let static_proxy_guard = GlobalWide(ie_cfg.lpszProxy);
            let static_bypass_guard = GlobalWide(ie_cfg.lpszProxyBypass);

            let auto_detect = ie_cfg.fAutoDetect != 0;
            let pac_url = unsafe { auto_url_guard.to_string_opt() }.filter(|s| !s.trim().is_empty());

            if auto_detect || pac_url.is_some() {
                let agent = to_wide("CyberAPI");
                let session_raw = unsafe {
                    WinHttpOpen(
                        agent.as_ptr(),
                        WINHTTP_ACCESS_TYPE_NO_PROXY,
                        ptr::null(),
                        ptr::null(),
                        0,
                    )
                };
                if !session_raw.is_null() {
                    let _session = WinHttpHandle(session_raw);
                    let wide_url = to_wide(&url_str);
                    let wide_pac = pac_url.as_deref().map(to_wide);
                    let mut opts = WINHTTP_AUTOPROXY_OPTIONS {
                        dwFlags: 0,
                        dwAutoDetectFlags: 0,
                        lpszAutoConfigUrl: wide_pac.as_ref().map_or(ptr::null(), |w| w.as_ptr()),
                        lpvReserved: ptr::null_mut(),
                        dwReserved: 0,
                        fAutoLogonIfChallenged: 1,
                    };
                    if auto_detect {
                        opts.dwFlags |= WINHTTP_AUTOPROXY_AUTO_DETECT;
                        opts.dwAutoDetectFlags = WINHTTP_AUTO_DETECT_TYPE_DHCP | WINHTTP_AUTO_DETECT_TYPE_DNS_A;
                    }
                    if wide_pac.is_some() {
                        opts.dwFlags |= WINHTTP_AUTOPROXY_CONFIG_URL;
                    }
                    let mut proxy_info = WINHTTP_PROXY_INFO::default();
                    let ok = unsafe {
                        WinHttpGetProxyForUrl(session_raw, wide_url.as_ptr(), &raw mut opts, &raw mut proxy_info)
                    };
                    let pi_proxy = GlobalWide(proxy_info.lpszProxy);
                    let pi_bypass = GlobalWide(proxy_info.lpszProxyBypass);
                    if ok != 0 {
                        if proxy_info.dwAccessType == WINHTTP_ACCESS_TYPE_NO_PROXY {
                            return Ok(None);
                        }
                        if let Some(bypass) = unsafe { pi_bypass.to_string_opt() }
                            && bypassed_windows(&host, &bypass)
                        {
                            return Ok(None);
                        }
                        if let Some(proxy_list) = unsafe { pi_proxy.to_string_opt() } {
                            return Ok(parse_winhttp_proxy_list(&proxy_list, &scheme));
                        }
                    }
                }
            }

            if let Some(bypass) = unsafe { static_bypass_guard.to_string_opt() }
                && bypassed_windows(&host, &bypass)
            {
                return Ok(None);
            }
            if let Some(proxy_list) = unsafe { static_proxy_guard.to_string_opt() } {
                return Ok(parse_winhttp_proxy_list(&proxy_list, &scheme));
            }
            Ok(None)
        })
        .await
        .map_err(|err| err.to_string())?
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use pretty_assertions::assert_eq;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn macos_pac_script_evaluates_proxy_socks_and_direct() {
        let pac = r#"
            function FindProxyForURL(url, host) {
                if (host === "direct.test") return "DIRECT";
                if (host === "socks.test") return "SOCKS 10.2.3.4:1080; DIRECT";
                return "PROXY 10.1.2.3:8080; DIRECT";
            }
        "#;
        let proxy_url = Url::parse("https://api.example.com/v1").expect("url");
        assert_eq!(
            eval_pac_script_for_test(pac, &proxy_url).await.expect("eval"),
            Some(Proxy {
                kind: cyberapi_core::proxy::ProxyKind::Http,
                host: "10.1.2.3".into(),
                port: 8080,
                auth: None,
            })
        );

        let socks_url = Url::parse("https://socks.test/items").expect("url");
        assert_eq!(
            eval_pac_script_for_test(pac, &socks_url).await.expect("eval"),
            Some(Proxy {
                kind: cyberapi_core::proxy::ProxyKind::Socks5 { remote_dns: true },
                host: "10.2.3.4".into(),
                port: 1080,
                auth: None,
            })
        );

        let direct_url = Url::parse("https://direct.test/ping").expect("url");
        assert_eq!(eval_pac_script_for_test(pac, &direct_url).await.expect("eval"), None);
    }

    #[tokio::test]
    async fn macos_pac_url_fetches_and_caches_script() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_bg = hits.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let hits_conn = hits_bg.clone();
                tokio::spawn(async move {
                    let svc = service_fn(move |_req| {
                        let hits_req = hits_conn.clone();
                        async move {
                            hits_req.fetch_add(1, Ordering::SeqCst);
                            let body = r#"function FindProxyForURL(url, host) { return "PROXY 127.0.0.1:9999"; }"#;
                            Ok::<_, std::convert::Infallible>(hyper::Response::new(http_body_util::Full::new(
                                bytes::Bytes::from_static(body.as_bytes()),
                            )))
                        }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), svc)
                        .await;
                });
            }
        });

        let pac_url = format!("http://{addr}/proxy-{}.pac", addr.port());
        let target = Url::parse("https://service.test/").expect("url");
        let first = fetch_and_eval_pac_for_test(&pac_url, &target, false)
            .await
            .expect("first eval");
        let second = fetch_and_eval_pac_for_test(&pac_url, &target, false)
            .await
            .expect("second cached eval");

        assert_eq!(
            first,
            Some(Proxy {
                kind: cyberapi_core::proxy::ProxyKind::Http,
                host: "127.0.0.1".into(),
                port: 9999,
                auth: None,
            })
        );
        assert_eq!(second, first);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }
}
