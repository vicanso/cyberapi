//! Talking to a proxy (#60) on a connection to it: an HTTP proxy's
//! `CONNECT` tunnel, or SOCKS5's greeting, sign-in and connect (RFC 1928,
//! RFC 1929). Which proxy, and whether at all, is `cyberapi_core::proxy`'s.

use base64::Engine as _;
use cyberapi_core::proxy::Proxy;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use url::Host;

/// The longest answer to a `CONNECT` read before giving up on it.
const CONNECT_ANSWER: usize = 16 * 1024;

/// `Proxy-Authorization` for the proxy's user, if it has one.
pub(crate) fn authorization(proxy: &Proxy) -> Option<String> {
    let (user, password) = proxy.auth.as_ref()?;
    let token = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
    Some(format!("Basic {token}"))
}

/// Asks an HTTP proxy for a tunnel to `authority` (`host:port`); the
/// connection is the tunnel once it agrees.
pub(crate) async fn tunnel<S>(stream: &mut S, proxy: &Proxy, authority: &str) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut asking = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if let Some(authorization) = authorization(proxy) {
        asking.push_str(&format!("Proxy-Authorization: {authorization}\r\n"));
    }
    asking.push_str("\r\n");
    let failed = |error: std::io::Error| error.to_string();
    stream.write_all(asking.as_bytes()).await.map_err(failed)?;
    // Byte by byte up to the blank line: what comes after it is the server's.
    let mut answer = Vec::new();
    while !answer.ends_with(b"\r\n\r\n") {
        if answer.len() >= CONNECT_ANSWER {
            return Err("the proxy's answer to CONNECT never ended".into());
        }
        let mut byte = [0u8];
        if stream.read(&mut byte).await.map_err(failed)? == 0 {
            return Err("the proxy closed the connection".into());
        }
        answer.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&answer);
    let status_line = text.lines().next().unwrap_or_default().trim().to_string();
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok());
    match status {
        Some(200..=299) => Ok(()),
        Some(407) => Err(format!("the proxy wants a user name and password ({status_line})")),
        _ => Err(format!("the proxy would not connect to {authority} ({status_line})")),
    }
}

/// Asks a SOCKS5 proxy to connect to `host:port`: a name goes as one, for
/// the proxy to find; an address as it is.
pub(crate) async fn socks5<S>(stream: &mut S, proxy: &Proxy, host: &Host<String>, port: u16) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let failed = |error: std::io::Error| error.to_string();
    let methods: &[u8] = if proxy.auth.is_some() { &[0x00, 0x02] } else { &[0x00] };
    let mut greeting = vec![0x05, methods.len() as u8];
    greeting.extend_from_slice(methods);
    stream.write_all(&greeting).await.map_err(failed)?;
    let mut chosen = [0u8; 2];
    stream.read_exact(&mut chosen).await.map_err(failed)?;
    if chosen[0] != 0x05 {
        return Err("the proxy does not speak SOCKS5".into());
    }
    match (chosen[1], &proxy.auth) {
        (0x00, _) => {}
        (0x02, Some((user, password))) => {
            let (user, password) = (user.as_bytes(), password.as_bytes());
            if user.len() > 255 || password.len() > 255 {
                return Err("the proxy's user name or password is longer than SOCKS5 takes".into());
            }
            let mut signing_in = vec![0x01, user.len() as u8];
            signing_in.extend_from_slice(user);
            signing_in.push(password.len() as u8);
            signing_in.extend_from_slice(password);
            stream.write_all(&signing_in).await.map_err(failed)?;
            let mut signed_in = [0u8; 2];
            stream.read_exact(&mut signed_in).await.map_err(failed)?;
            if signed_in[1] != 0x00 {
                return Err("the proxy refused the user name and password".into());
            }
        }
        (0x02, None) => return Err("the proxy wants a user name and password".into()),
        _ => return Err("the proxy takes none of the ways this signs in".into()),
    }
    let mut connect = vec![0x05, 0x01, 0x00];
    match host {
        Host::Ipv4(address) => {
            connect.push(0x01);
            connect.extend_from_slice(&address.octets());
        }
        Host::Ipv6(address) => {
            connect.push(0x04);
            connect.extend_from_slice(&address.octets());
        }
        Host::Domain(name) => {
            if name.len() > 255 {
                return Err("the host name is longer than SOCKS5 takes".into());
            }
            connect.push(0x03);
            connect.push(name.len() as u8);
            connect.extend_from_slice(name.as_bytes());
        }
    }
    connect.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&connect).await.map_err(failed)?;
    let mut reply = [0u8; 4];
    stream.read_exact(&mut reply).await.map_err(failed)?;
    if reply[1] != 0x00 {
        let why = match reply[1] {
            0x01 => "it failed",
            0x02 => "its rules do not allow it",
            0x03 => "the network is unreachable",
            0x04 => "the host is unreachable",
            0x05 => "the connection was refused",
            0x06 => "it took too long",
            0x07 => "it does not take that command",
            0x08 => "it does not take that kind of address",
            _ => "it did not say why",
        };
        return Err(format!("the proxy could not connect: {why}"));
    }
    // Where the proxy connected from, which is no matter here.
    let bound = match reply[3] {
        0x01 => 4,
        0x04 => 16,
        0x03 => {
            let mut length = [0u8];
            stream.read_exact(&mut length).await.map_err(failed)?;
            usize::from(length[0])
        }
        _ => return Err("the proxy answered with an address it should not".into()),
    };
    let mut rest = vec![0u8; bound + 2];
    stream.read_exact(&mut rest).await.map_err(failed)?;
    Ok(())
}
