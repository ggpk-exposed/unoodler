use serde::Deserialize;
use worker::*;

#[event(fetch)]
async fn fetch(req: Request, _env: Env, _ctx: Context) -> Result<Response> {
    console_error_panic_hook::set_once();
    handler(
        req.query()?,
        req.method(),
        req.headers().get("accept-encoding")?,
    )
    .await
}

#[derive(Deserialize)]
struct Params {
    url: String,
    offset: usize,
    compressed: usize,
    extracted: usize,
    #[serde(default)]
    raw: bool,
}

async fn handler(
    Params {
        url,
        offset,
        compressed,
        extracted,
        raw,
    }: Params,
    method: Method,
    accept_encoding: Option<String>,
) -> Result<Response> {
    if !(url.starts_with("https://patch.poecdn.com/")
        || url.starts_with("https://patch-poe2.poecdn.com/"))
    {
        return Response::error(format!("host not recognized: {}", url), 400);
    }

    if method == Method::Head {
        let headers = head(&url).await?;
        return Ok(Response::builder().with_headers(headers).empty());
    } else if method != Method::Get {
        return Response::error("GET, HEAD only", 405);
    }

    let response = get_data(url, offset, compressed)
        .await
        .map_err(|e| Error::Internal(format!("Upstream error {}", e).into()))?;

    let content_range = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok());

    if !content_range.is_some_and(|r| {
        r.starts_with(format!("bytes {}-{}", offset, offset + compressed - 1).as_str())
    }) {
        return Response::error(format!("range header ignored: {:?}", content_range), 500);
    }

    let headers = copy_headers(&response, accept_encoding);

    if raw {
        return ResponseBuilder::new().with_headers(headers).from_bytes(
            response
                .bytes()
                .await
                .map_err(|e| Error::Internal(format!("Download error {}", e).into()))?
                .into(),
        );
    }

    let mut result = vec![0; extracted];
    match oozextract::Extractor::new()
        .read_from_stream(&mut response.bytes_stream(), None, &mut result)
        .await
    {
        Ok(_) => ResponseBuilder::new()
            .with_headers(headers)
            .from_bytes(result),
        Err(e) => Response::error(format!("Decompression error: {}", e), 500),
    }
}

async fn head(url: &String) -> Result<Headers> {
    Ok(copy_headers(
        &reqwest::Client::new()
            .head(url)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Upstream error {}", e).into()))?,
        None,
    ))
}

fn copy_headers(response: &reqwest::Response, accept_encoding: Option<String>) -> Headers {
    let mut headers = Headers::new();
    for header in ["last-modified", "etag", "cache-control", "expires", "date"] {
        response
            .headers()
            .get(header)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| headers.set(header, v).ok());
    }

    // both the incoming accept-encoding header and the actual encoding of the outgoing file are modified by cloudflare.
    // just need to add the incoming header to our output headers to enable cf to compress the data
    // https://community.cloudflare.com/t/worker-doesnt-return-gzip-brotli-compressed-data/337644/3
    if let Some(encoding) = accept_encoding
        .as_ref()
        .and_then(|v| v.split(',').map(str::trim).next())
    {
        headers.set("content-encoding", encoding).ok();
    }
    headers
}

async fn get_data(
    url: String,
    offset: usize,
    compressed: usize,
) -> reqwest::Result<reqwest::Response> {
    Ok(reqwest::Client::builder()
        .build()?
        .get(url)
        .header(
            reqwest::header::RANGE,
            format!("bytes={}-{}", offset, offset + compressed - 1),
        )
        .send()
        .await?)
}
