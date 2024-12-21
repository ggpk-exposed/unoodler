use reqwest::header::HeaderMap;
use serde::Deserialize;
use worker::*;

#[event(fetch)]
async fn fetch(req: Request, _env: Env, _ctx: Context) -> Result<Response> {
    console_error_panic_hook::set_once();
    handler(req.query()?).await
}

#[derive(Deserialize)]
struct Params {
    url: String,
    offset: usize,
    compressed: usize,
    extracted: usize,
}

async fn handler(
    Params {
        url,
        offset,
        compressed,
        extracted,
    }: Params,
) -> Result<Response> {
    if !(url.starts_with("https://patch.poecdn.com/")
        || url.starts_with("https://patch-poe2.poecdn.com/"))
    {
        return Response::error(format!("host not recognized: {}", url), 400);
    }

    let response = get_data(url, offset, compressed).await;

    if let Err(e) = response {
        return Response::error(format!("Failed to fetch {}", e), 500);
    }
    let response = response.unwrap();

    let content_range = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok());

    if !content_range.is_some_and(|r| {
        r.starts_with(format!("bytes {}-{}", offset, offset + compressed - 1).as_str())
    }) {
        return Response::error(format!("range header ignored: {:?}", content_range), 500);
    }

    let mut headers = Headers::new();
    for header in ["last-modified", "etag", "cache-control", "expires", "date"] {
        response
            .headers()
            .get(header)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| headers.set(header, v).ok());
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
