use serde::Deserialize;
use worker::*;

#[event(fetch)]
async fn fetch(req: Request, _env: Env, _ctx: Context) -> Result<Response> {
    console_error_panic_hook::set_once();
    handler(
        req.query()?,
        req.method(),
        req.headers().get("if-none-match")?,
    )
    .await
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
    method: Method,
    if_none_match: Option<String>,
) -> Result<Response> {
    if !(url.starts_with("https://patch.poecdn.com/")
        || url.starts_with("https://patch-poe2.poecdn.com/"))
    {
        return Response::error(format!("host not recognized: {}", url), 400);
    }

    if let Some(matches) = if_none_match.as_ref().and_then(parse_etags) {
        let headers = head(&url).await?;
        if let Some(etag) = headers.get("ETag")?.as_ref().and_then(parse_etags) {
            for i in etag {
                if matches.contains(&i) {
                    return Ok(Response::builder()
                        .with_headers(headers)
                        .with_status(304)
                        .empty());
                }
            }
        }
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
        return Err(Error::Internal(
            format!("range header ignored: {:?}", content_range).into(),
        ));
    }

    let headers = copy_headers(&response);

    let mut result = vec![0; extracted];
    match oozextract::Extractor::new()
        .read_from_stream(&mut response.bytes_stream(), None, &mut result)
        .await
    {
        Ok(_) => ResponseBuilder::new()
            .with_headers(headers)
            .from_bytes(result),
        Err(e) => Err(Error::Internal(
            format!("Decompression error: {}", e).into(),
        )),
    }
}

fn parse_etags(m: &String) -> Option<Vec<&str>> {
    // get the contents of each opaque-tag, assuming the header follows
    // https://www.rfc-editor.org/rfc/rfc7232#appendix-C
    let etags = m.split('"').skip(1).step_by(2).collect::<Vec<_>>();
    if etags.is_empty() {
        None
    } else {
        Some(etags)
    }
}

async fn head(url: &String) -> Result<Headers> {
    Ok(copy_headers(
        &reqwest::Client::new()
            .head(url)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Upstream error {}", e).into()))?,
    ))
}

fn copy_headers(response: &reqwest::Response) -> Headers {
    let mut headers = Headers::new();
    for header in ["last-modified", "etag", "cache-control", "expires", "date"] {
        response
            .headers()
            .get(header)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| headers.set(header, v).ok());
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
