#![feature(proc_macro_hygiene)]

use windmill::*;

use http_types::{headers::HeaderName, mime, Method, Mime, StatusCode};
use serde::Deserialize;
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

#[derive(Deserialize, Debug)]
struct ExampleRequest {
    foo: String,
}

fn main() {
    let mut router = Router::new();
    let config = Config::new("127.0.0.1:4000");

    #[rustfmt::skip]
    router.add(Method::Get, route!(/"example"/id), ___example_route);

    if let Err(e) = Server::new(config).run(router) {
        println!("{}", e);
    }
}

struct Auth {
    user_id: u64,
    token: String,
}

fn parse_header<'h>(req: &'h http_types::Request) -> Result<String, Error> {
    use std::str::FromStr;
    let header_name =
        HeaderName::from_str("authorization").map_err(|header_name| Error {
            code: StatusCode::InternalServerError,
            msg: json!("bad header name"),
        })?;

    let header = req.header(&header_name).ok_or_else(|| Error {
        code: StatusCode::BadRequest,
        msg: json!("authorization required"),
    })?;

    let header = header.first().as_ref().unwrap().to_string();

    Ok(header)
}


impl Service for Auth {
    type Fut = ServiceFuture<Self>;
    fn call(mut req: http_types::Request, params: Params) -> Self::Fut {
        Box::pin(async move {
            let header = parse_header(&req)?;

            Ok((req, params, Self {
                user_id: 1,
                token: header,
            }))
        })
    }
}

struct Body<T> {
    inner: Option<T>,
}

impl<T: for<'de> Deserialize<'de> + std::fmt::Debug> Service for Body<T> {
    type Fut = ServiceFuture<Self>;
    fn call(mut req: http_types::Request, params: Params) -> Self::Fut {
        Box::pin(async move {
            let body: Option<T> =
                serde_json::from_slice(&read_body(&mut req).await).unwrap_or_else(|_| None);

            Ok((req, params, Body { inner: body }))
        })
    }
}

#[endpoint]
async fn example_route(
    auth: Auth,
    body: Body<ExampleRequest>,
) -> Result<http_types::Response, Error> {

    let body = body.inner.ok_or_else(|| Error {
        code: StatusCode::BadRequest,
        msg: serde_json::json!("body required"),
    })?;

    dbg!(&body);
    //    use std::str::FromStr;
    //    let id = u64::from_str(req.params().get("id").ok_or_else(|| Error {
    //        code: StatusCode::InternalServerError,
    //        msg: serde_json::json!("param does not exist"),
    //    })?)
    //    .map_err(|e| Error {
    //        code: StatusCode::BadRequest,
    //        msg: serde_json::json!(format!("{}", e)),
    //    })?;
    //
    //    //    let body = req.body().ok_or_else(|| Error {
    //    //        code: StatusCode::BadRequest,
    //    //        msg: serde_json::json!("body required"),
    //    //    })?;
    //
    //    let auth = req
    //        .context()
    //        .get("auth")
    //        .ok_or_else(|| Error {
    //            code: StatusCode::InternalServerError,
    //            msg: serde_json::json!("context value auth does not exist"),
    //        })?
    //        .downcast_ref::<http_types::headers::HeaderValue>()
    //        .ok_or_else(|| Error {
    //            code: StatusCode::InternalServerError,
    //            msg: serde_json::json!("invalid cast to HeaderValue"),
    //        })?;
    //
    Ok(http_types::Response::new(StatusCode::Ok))
}
