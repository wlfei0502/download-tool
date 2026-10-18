use crate::task::{require_http_url, NewTaskRequest};
use url::Url;

/// Parse `dltool://add?url=...&fileName=...&size=...`.
///
/// The inner download URL must be `http://` or `https://`. `fileName` is optional; an omitted
/// or unsafe name is taken from the download URL path. `referrer` and
/// `userAgent` are optional and match the local API body.
pub fn parse_dltool_url(input: &str) -> Result<NewTaskRequest, String> {
    let input = input.trim().trim_matches('"');
    let parsed = Url::parse(input).map_err(|err| format!("invalid dltool url: {err}"))?;
    if parsed.scheme() != "dltool" {
        return Err(format!(
            "expected dltool://add, found scheme {}",
            parsed.scheme()
        ));
    }
    if parsed.host_str() != Some("add") || !matches!(parsed.path(), "" | "/") {
        return Err("expected dltool://add".into());
    }

    let mut url = None;
    let mut file_name = None;
    let mut size = None;
    let mut referrer = None;
    let mut user_agent = None;
    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "url" if url.is_none() => url = Some(value.into_owned()),
            "fileName" if file_name.is_none() => file_name = Some(value.into_owned()),
            "size" if size.is_none() => size = value.parse().ok(),
            "referrer" if referrer.is_none() => {
                let value = value.into_owned();
                if !value.is_empty() {
                    referrer = Some(value);
                }
            }
            "userAgent" if user_agent.is_none() => {
                let value = value.into_owned();
                if !value.is_empty() {
                    user_agent = Some(value);
                }
            }
            _ => {}
        }
    }

    let url = url.ok_or_else(|| "url required".to_string())?;
    require_http_url(&url)?;
    let file_name = match file_name {
        Some(name) => sanitize_file_name(&name, &url),
        None => suggested_file_name(&url),
    };

    Ok(NewTaskRequest {
        url,
        file_name,
        size,
        referrer,
        user_agent,
    })
}

/// Keep `name` when it is a single path segment. Otherwise use the download URL's file name.
pub fn sanitize_file_name(name: &str, url: &str) -> String {
    if safe_file_name(name) {
        name.to_string()
    } else {
        suggested_file_name(url)
    }
}

/// Last path segment of an http URL, or `"download"` when that segment is not a file name.
pub fn suggested_file_name(url: &str) -> String {
    let Ok(parsed) = Url::parse(url) else {
        return "download".into();
    };
    let name = parsed
        .path_segments()
        .and_then(|segments| segments.rev().find(|segment| !segment.is_empty()))
        .unwrap_or("");
    let name = percent_decode(name);
    if safe_file_name(&name) {
        name
    } else {
        "download".into()
    }
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Some(byte) = hex_byte(bytes[index + 1], bytes[index + 2]) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_byte(high: u8, low: u8) -> Option<u8> {
    Some((hex_value(high)? << 4) | hex_value(low)?)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
}
