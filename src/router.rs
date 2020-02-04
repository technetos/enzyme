use crate::context::Context;
use crate::params::Params;
use crate::result::WebResult;
use http_types::{headers, Method, Request, Response, StatusCode};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::sync::Arc;
use std::{collections::HashMap, pin::Pin};

pub struct StaticSegment {
    value: &'static str,
    position: usize,
}

pub struct DynamicSegment {
    name: &'static str,
    position: usize,
}

pub struct Route {
    static_segments: Vec<StaticSegment>,
    dynamic_segments: Vec<DynamicSegment>,
    handler: Option<Box<dyn Fn(Request, Params) -> Pin<Box<dyn Future<Output = Response>>>>>,
}

pub struct Router {
    table: HashMap<Method, Vec<Route>>,
}

pub trait HttpStatusCode {
    fn code(&self) -> StatusCode;
}

pub trait Endpoint<Error, Req, Res>: 'static + Copy
where
    Error: HttpStatusCode + 'static,
    Req: for<'de> Deserialize<'de> + 'static,
    Res: Serialize + 'static,
{
    type Fut: Future<Output = Result<Res, Error>>;
    fn call(&self, req: Req, params: Params) -> Self::Fut;
}

impl<Error, Req, Res, F, G> Endpoint<Error, Req, Res> for F
where
    Error: HttpStatusCode + 'static,
    Req: for<'de> Deserialize<'de> + 'static,
    Res: Serialize + 'static,
    G: Future<Output = Result<Res, Error>> + 'static,
    F: Fn(Req, Params) -> G + 'static + Copy,
{
    type Fut = Pin<Box<Future<Output = Result<Res, Error>>>>;
    fn call(&self, req: Req, params: Params) -> Self::Fut {
        let fut = (self)(req, params);
        Box::pin(async move { fut.await })
    }
}

impl Router {
    pub fn new() -> Self {
        Router {
            table: HashMap::new(),
        }
    }

    pub fn add<Error, Req, Res>(
        &mut self,
        method: Method,
        mut route: Route,
        endpoint: impl Endpoint<Error, Req, Res>,
    ) where
        Error: HttpStatusCode + 'static,
        Req: for<'de> Deserialize<'de> + Default + 'static,
        Res: Serialize + 'static + Default,
    {
        let entry = self
            .table
            .entry(method)
            .or_insert_with(|| Vec::<Route>::new());

        let handler =
            move |mut req: Request, params: Params| -> Pin<Box<dyn Future<Output = Response>>> {
                use async_std::prelude::*;

                let fut = async move {
                    let has_body = req
                        .header(&headers::CONTENT_LENGTH)
                        .map(|values| values.first().map(|value| value.as_str() == "0"))
                        .flatten()
                        .unwrap_or_else(|| false);

                    let req: Req = if has_body {
                        let mut body = vec![];
                        req.read_to_end(&mut body).await.unwrap();

                        serde_json::from_slice(&body).unwrap()
                    } else {
                        Req::default()
                    };

                    let res: Res = match endpoint.call(req, params).await {
                        Ok(body) => body,
                        Err(e) => return Response::new(e.code()),
                    };

                    let res_bytes = serde_json::to_vec(&res).unwrap();
                    let mut res = Response::new(StatusCode::Ok);
                    res.set_body(res_bytes);

                    res
                };
                Box::pin(fut)
            };

        route.handler = Some(Box::new(handler));
        entry.push(route);
    }

    pub(crate) fn lookup(
        self: Arc<Self>,
        req: Request,
    ) -> Box<dyn Future<Output = Response> + Unpin> {
        let method = req.method();
        let raw_route = RawRoute::from_path(req.url().path().into());
        let maybe_route = if let Some(routes) = self.table.get(&method) {
            routes
                .iter()
                .filter(|route| paths_match(route, &raw_route))
                .nth(0)
        } else {
            return Box::new(Box::pin(not_found()));
        };

        if let Some(route) = maybe_route {
            let params = route.dynamic_segments.iter().fold(
                HashMap::new(),
                |mut params, dynamic_segment| {
                    params.insert(
                        dynamic_segment.name,
                        raw_route.raw_segments[dynamic_segment.position]
                            .value
                            .into(),
                    );
                    params
                },
            );

            Box::new((route.handler.as_ref().unwrap())(req, params))
        } else {
            Box::new(Box::pin(not_found()))
        }
    }
}

fn paths_match(route: &Route, raw_route: &RawRoute) -> bool {
    if raw_route.raw_segments.len() == route.static_segments.len() + route.dynamic_segments.len() {
        let static_matches = || {
            route
                .static_segments
                .iter()
                .fold(true, |is_match, static_segment| {
                    is_match && (&raw_route.raw_segments[static_segment.position] == static_segment)
                })
        };

        let dynamic_matches = || {
            route
                .dynamic_segments
                .iter()
                .fold(true, |is_match, dynamic_segment| {
                    is_match
                        && (&raw_route.raw_segments[dynamic_segment.position] == dynamic_segment)
                })
        };

        static_matches() && dynamic_matches()
    } else {
        false
    }
}

async fn not_found() -> Response {
    use serde_json::json;

    let mut res = Response::new(StatusCode::NotFound);
    res.set_body(serde_json::to_vec(&json!("not found")).unwrap());
    res
}

pub(crate) struct RawSegment<'s> {
    value: &'s str,
    position: usize,
}

pub(crate) struct RawRoute<'s> {
    pub raw_segments: Vec<RawSegment<'s>>,
}

impl<'s> RawRoute<'s> {
    pub(crate) fn from_path(path: &'s str) -> Self {
        Self {
            raw_segments: path
                .split("/")
                .skip(1)
                .enumerate()
                .map(|(i, segment)| RawSegment {
                    value: segment,
                    position: i,
                })
                .collect(),
        }
    }
}

impl<'s> PartialEq<RawSegment<'s>> for StaticSegment {
    fn eq(&self, other: &RawSegment) -> bool {
        self.position == other.position && self.value == other.value
    }
}

impl<'s> PartialEq<RawSegment<'s>> for DynamicSegment {
    fn eq(&self, other: &RawSegment) -> bool {
        self.position == other.position
    }
}

impl<'s> PartialEq<StaticSegment> for RawSegment<'s> {
    fn eq(&self, other: &StaticSegment) -> bool {
        other == self
    }
}

impl<'s> PartialEq<DynamicSegment> for RawSegment<'s> {
    fn eq(&self, other: &DynamicSegment) -> bool {
        other == self
    }
}

#[test]
fn test() {
    use crate::macros::route;
    use serde::{Deserialize, Serialize};

    pub struct Error {
        code: StatusCode,
    }

    impl HttpStatusCode for Error {
        fn code(&self) -> StatusCode {
            self.code
        }
    }

    let mut router = Router::new();

    #[derive(Deserialize, Default)]
    struct ExampleRequest {
        image_orientation: String,
    }

    #[derive(Serialize, Default)]
    struct ExampleResponse;

    async fn example_route(req: ExampleRequest, params: Params) -> Result<ExampleResponse, Error> {
        Ok(ExampleResponse)
    }

    #[derive(Deserialize, Default)]
    struct ExampleRequest2;

    #[derive(Serialize, Default)]
    struct ExampleResponse2;

    async fn another_route(req: ExampleRequest2, params: Params) -> Result<ExampleResponse2, Error> {
        Err(Error {
            code: StatusCode::Unauthorized,
        })
    }
    
    router.add(Method::Get, route!(/"images"/image_id), example_route);
    router.add(Method::Get, route!(/"foo"), another_route);
}
