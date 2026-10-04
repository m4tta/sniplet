use std::time::Duration;

use image::{ImageEncoder, RgbaImage};
use s3::{AddressingStyle, Auth, BlockingClient};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{PlatformError, Result, sanitize_filename};

const PNG_CONTENT_TYPE: &str = "image/png";
// AWS Signature Version 4 caps presigned URLs at seven days.
const PRESIGNED_GET_LIFETIME: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// An explicit cloud destination. Authentication material is deliberately not
/// represented here: S3 credentials are read from the standard AWS environment
/// variables only when [`upload_image`] is called.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CloudUploadConfig {
    /// A one-use URL obtained from an external service.
    PresignedPut {
        url: String,
        public_url: Option<String>,
    },
    /// An S3 or S3-compatible object store.
    S3 {
        bucket: String,
        region: String,
        endpoint: Option<String>,
        key_prefix: String,
        public_base_url: Option<String>,
    },
}

/// The location produced by a completed upload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadedImage {
    pub url: String,
    pub key: String,
}

/// Encode and upload an RGBA image as PNG.
///
/// This function is the only operation that performs network I/O. Constructing
/// or loading a [`CloudUploadConfig`] never uploads anything.
pub fn upload_image(
    image: &RgbaImage,
    key: &str,
    config: &CloudUploadConfig,
) -> Result<UploadedImage> {
    let filename = png_filename(key)?;
    let png = encode_png(image)?;

    match config {
        CloudUploadConfig::PresignedPut { url, public_url } => {
            upload_presigned_put(png, &filename, url, public_url.as_deref())
        }
        CloudUploadConfig::S3 {
            bucket,
            region,
            endpoint,
            key_prefix,
            public_base_url,
        } => {
            let auth = Auth::from_env().map_err(s3_error)?;
            upload_s3_with_auth(
                png,
                &filename,
                S3Destination {
                    bucket,
                    region,
                    endpoint: endpoint.as_deref(),
                    key_prefix,
                    public_base_url: public_base_url.as_deref(),
                },
                auth,
            )
        }
    }
}

fn encode_png(image: &RgbaImage) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(PlatformError::Image)?;
    Ok(bytes)
}

fn png_filename(requested: &str) -> Result<String> {
    if requested.trim().is_empty() {
        return Err(invalid_config("upload key must not be empty"));
    }
    let mut filename = sanitize_filename(requested);
    if filename.is_empty() {
        return Err(invalid_config("upload key has no usable characters"));
    }
    if !filename.to_ascii_lowercase().ends_with(".png") {
        filename.push_str(".png");
    }
    Ok(filename)
}

fn upload_presigned_put(
    png: Vec<u8>,
    key: &str,
    upload_url: &str,
    public_url: Option<&str>,
) -> Result<UploadedImage> {
    let mut parsed_upload_url = parse_http_url(upload_url, "presigned PUT URL")?;

    match ureq::put(parsed_upload_url.as_str())
        .header("Content-Type", PNG_CONTENT_TYPE)
        .send(png.as_slice())
    {
        Ok(_) => {}
        Err(ureq::Error::StatusCode(status)) => {
            return Err(PlatformError::UploadHttpStatus { status });
        }
        Err(_) => return Err(PlatformError::UploadTransport),
    }

    let url = if let Some(public_url) = public_url {
        parse_http_url(public_url, "public URL")?.to_string()
    } else {
        // The signature itself is both sensitive and temporary. If no public
        // URL is supplied, return the stable object URL without its query.
        parsed_upload_url.set_query(None);
        parsed_upload_url.set_fragment(None);
        parsed_upload_url.to_string()
    };

    Ok(UploadedImage {
        url,
        key: key.to_owned(),
    })
}

struct S3Destination<'a> {
    bucket: &'a str,
    region: &'a str,
    endpoint: Option<&'a str>,
    key_prefix: &'a str,
    public_base_url: Option<&'a str>,
}

fn upload_s3_with_auth(
    png: Vec<u8>,
    filename: &str,
    destination: S3Destination<'_>,
    auth: Auth,
) -> Result<UploadedImage> {
    if destination.bucket.trim().is_empty() {
        return Err(invalid_config("S3 bucket must not be empty"));
    }
    if destination.region.trim().is_empty() {
        return Err(invalid_config("S3 region must not be empty"));
    }

    let key = object_key(destination.key_prefix, filename)?;
    let endpoint = destination
        .endpoint
        .map(str::to_owned)
        .unwrap_or_else(|| format!("https://s3.{}.amazonaws.com", destination.region));
    parse_http_url(&endpoint, "S3 endpoint")?;

    let client = BlockingClient::builder(&endpoint)
        .map_err(s3_error)?
        .region(destination.region)
        .auth(auth)
        .addressing_style(AddressingStyle::Path)
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(s3_error)?;

    client
        .objects()
        .put(destination.bucket, &key)
        .content_type(PNG_CONTENT_TYPE)
        .map_err(s3_error)?
        .body_bytes(png)
        .send()
        .map_err(s3_error)?;

    let url = if let Some(public_base_url) = destination.public_base_url {
        public_object_url(public_base_url, &key)?
    } else {
        client
            .objects()
            .presign_get(destination.bucket, &key)
            .expires_in(PRESIGNED_GET_LIFETIME)
            .map_err(s3_error)?
            .build()
            .map_err(s3_error)?
            .url
            .to_string()
    };

    Ok(UploadedImage { url, key })
}

fn object_key(prefix: &str, filename: &str) -> Result<String> {
    let prefix = prefix.trim_matches('/');
    if prefix.is_empty() {
        return Ok(filename.to_owned());
    }
    if prefix.contains('\\')
        || prefix.chars().any(char::is_control)
        || prefix.split('/').any(|segment| {
            segment.is_empty() || segment == "." || segment == ".." || segment.trim().is_empty()
        })
    {
        return Err(invalid_config("S3 key prefix contains an unsafe segment"));
    }
    Ok(format!("{prefix}/{filename}"))
}

fn public_object_url(base: &str, key: &str) -> Result<String> {
    let mut url = parse_http_url(base, "public base URL")?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| invalid_config("public base URL cannot contain path segments"))?;
        segments.pop_if_empty();
        for segment in key.split('/') {
            segments.push(segment);
        }
    }
    Ok(url.to_string())
}

fn parse_http_url(value: &str, name: &str) -> Result<Url> {
    let url =
        Url::parse(value).map_err(|_| invalid_config(format!("{name} is not a valid URL")))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(invalid_config(format!(
            "{name} must be an HTTP or HTTPS URL"
        )));
    }
    Ok(url)
}

fn invalid_config(message: impl Into<String>) -> PlatformError {
    PlatformError::InvalidCloudUploadConfig(message.into())
}

fn s3_error(error: s3::Error) -> PlatformError {
    PlatformError::S3(Box::new(error))
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    use image::{GenericImageView, Rgba};
    use s3::Credentials;

    use super::*;
    use crate::Settings;

    fn fixture() -> RgbaImage {
        RgbaImage::from_pixel(3, 2, Rgba([20, 40, 60, 255]))
    }

    fn mock_server(status: u16) -> (String, thread::JoinHandle<(String, Vec<u8>)>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_request(&mut stream, status)
        });
        (format!("http://{address}"), handle)
    }

    fn read_request(stream: &mut TcpStream, status: u16) -> (String, Vec<u8>) {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut received = Vec::new();
        let header_end = loop {
            let mut chunk = [0_u8; 4096];
            let read = stream.read(&mut chunk).unwrap();
            assert_ne!(read, 0, "connection closed before request headers");
            received.extend_from_slice(&chunk[..read]);
            if let Some(index) = received.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                break index + 4;
            }
        };

        let head = String::from_utf8(received[..header_end].to_vec()).unwrap();
        let lowercase_head = head.to_ascii_lowercase();
        if lowercase_head.contains("expect: 100-continue") {
            stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").unwrap();
        }
        let content_length = lowercase_head
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .map(str::trim)
            .map(|value| value.parse::<usize>().unwrap())
            .unwrap_or(0);
        while received.len() - header_end < content_length {
            let mut chunk = [0_u8; 4096];
            let read = stream.read(&mut chunk).unwrap();
            assert_ne!(read, 0, "connection closed before request body");
            received.extend_from_slice(&chunk[..read]);
        }

        let reason = if status == 200 { "OK" } else { "Forbidden" };
        write!(
            stream,
            "HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        (
            head,
            received[header_end..header_end + content_length].to_vec(),
        )
    }

    fn assert_png(head: &str, body: &[u8]) {
        assert!(
            head.to_ascii_lowercase()
                .contains("content-type: image/png")
        );
        assert_eq!(&body[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(image::load_from_memory(body).unwrap().dimensions(), (3, 2));
    }

    #[test]
    fn presigned_put_uploads_png_and_returns_public_url() {
        let (server, handle) = mock_server(200);
        let secret = "do-not-emit";
        let config = CloudUploadConfig::PresignedPut {
            url: format!("{server}/upload/object.png?token={secret}"),
            public_url: Some("https://cdn.example.test/shots/object.png".to_owned()),
        };

        let uploaded = upload_image(&fixture(), "object", &config).unwrap();
        let (head, body) = handle.join().unwrap();

        assert!(head.starts_with(&format!("PUT /upload/object.png?token={secret} ")));
        assert_png(&head, &body);
        assert_eq!(uploaded.key, "object.png");
        assert_eq!(uploaded.url, "https://cdn.example.test/shots/object.png");
        assert!(!format!("{uploaded:?}").contains(secret));
    }

    #[test]
    fn presigned_put_reports_status_without_leaking_url() {
        let (server, handle) = mock_server(403);
        let secret = "private-signature";
        let config = CloudUploadConfig::PresignedPut {
            url: format!("{server}/object.png?signature={secret}"),
            public_url: None,
        };

        let error = upload_image(&fixture(), "object.png", &config).unwrap_err();
        handle.join().unwrap();

        assert!(matches!(
            error,
            PlatformError::UploadHttpStatus { status: 403 }
        ));
        assert!(!error.to_string().contains(secret));
    }

    #[test]
    fn s3_upload_is_signed_path_style_and_url_encoded() {
        let (server, handle) = mock_server(200);
        let credentials = Credentials::new("test-access", "test-secret").unwrap();
        let uploaded = upload_s3_with_auth(
            encode_png(&fixture()).unwrap(),
            "My shot.png",
            S3Destination {
                bucket: "test-bucket",
                region: "us-east-1",
                endpoint: Some(&server),
                key_prefix: "team captures/2026",
                public_base_url: Some("https://cdn.example.test/base/"),
            },
            Auth::Static(credentials),
        )
        .unwrap();
        let (head, body) = handle.join().unwrap();
        let lowercase_head = head.to_ascii_lowercase();

        assert!(head.starts_with("PUT /test-bucket/team%20captures/2026/My%20shot.png "));
        assert!(lowercase_head.contains("authorization: aws4-hmac-sha256"));
        assert!(!head.contains("test-secret"));
        assert_png(&head, &body);
        assert_eq!(uploaded.key, "team captures/2026/My shot.png");
        assert_eq!(
            uploaded.url,
            "https://cdn.example.test/base/team%20captures/2026/My%20shot.png"
        );
        assert!(!format!("{uploaded:?}").contains("test-access"));
    }

    #[test]
    fn upload_settings_never_contain_credentials() {
        let settings = Settings {
            cloud_upload: Some(CloudUploadConfig::S3 {
                bucket: "screenshots".to_owned(),
                region: "us-west-2".to_owned(),
                endpoint: None,
                key_prefix: "sniplet".to_owned(),
                public_base_url: None,
            }),
            ..Settings::default()
        };

        let json = serde_json::to_string(&settings).unwrap();
        assert!(!json.contains("access_key"));
        assert!(!json.contains("secret"));
        assert!(!json.contains("session_token"));
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), settings);
    }

    #[test]
    fn unsafe_s3_prefix_is_rejected_before_network_access() {
        let error = object_key("safe/../escape", "shot.png").unwrap_err();
        assert!(matches!(error, PlatformError::InvalidCloudUploadConfig(_)));
    }
}
