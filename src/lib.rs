use serde::Deserialize;
use std::num::ParseIntError;
use std::str::FromStr;
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
    offset: String,
    compressed: String,
    extracted: String,
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

    let mut out_size = 0;
    let mut start = None;
    let mut end = None;
    let blocks: std::result::Result<Vec<_>, Response> = offset
        .split(",")
        .map(usize::from_str)
        .zip(compressed.split(",").map(usize::from_str))
        .zip(extracted.split(",").map(usize::from_str))
        .map(|((offset, compressed), extracted)| Ok((offset?, compressed?, extracted?)))
        .map(|block: std::result::Result<_, ParseIntError>| {
            let (offset, compressed, extracted) = block.map_err(|e| {
                ResponseBuilder::new()
                    .with_status(400)
                    .fixed(format!("Bad numeric parameter: {:?}", e.kind()).into_bytes())
            })?;
            if let Some(prev_end) = end {
                if offset != prev_end {
                    return Err(ResponseBuilder::new()
                        .with_status(400)
                        .fixed("Non-contiguous blocks ending".as_bytes().to_vec()));
                }
            }
            if start.is_none() {
                start = Some(offset);
            }
            end = Some(offset + compressed);
            out_size += extracted;
            Ok(extracted)
        })
        .collect();

    let blocks = match blocks {
        Ok(blocks) => blocks,
        Err(response) => return Ok(response),
    };

    let (start, end) = match (start, end) {
        (Some(start), Some(end)) => (start, end - 1),
        _ => return Response::error("missing parameters", 400),
    };

    let response = get_data(url, start, end)
        .await
        .map_err(|e| Error::Internal(format!("Upstream error {}", e).into()))?;

    let content_range = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok());

    if !content_range.is_some_and(|r| r.starts_with(format!("bytes {}-{}", start, end).as_str())) {
        return Response::error(
            format!(
                "range header (bytes {}-{}) ignored: {:?}",
                start, end, content_range
            ),
            500,
        );
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

    let mut input = response.bytes_stream();
    let mut output = vec![0; out_size];
    let mut i = 0;
    let mut remainder = None;
    for extracted in blocks {
        match oozextract::Extractor::new()
            .read_from_stream(
                &mut input,
                remainder,
                output.get_mut(i..i + extracted).unwrap(),
            )
            .await
        {
            Ok((_, r)) => remainder = r,
            Err(e) => return Response::error(format!("Decompression error: {}", e), 500),
        }
        i += extracted;
    }

    ResponseBuilder::new()
        .with_headers(headers)
        .from_bytes(output)
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

async fn get_data(url: String, start: usize, end: usize) -> reqwest::Result<reqwest::Response> {
    Ok(reqwest::Client::builder()
        .build()?
        .get(url)
        .header(reqwest::header::RANGE, format!("bytes={}-{}", start, end))
        .send()
        .await?)
}
