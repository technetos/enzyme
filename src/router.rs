use crate::{endpoint::AsyncResponse, params::Params};
use http_types::{Method, Request, Response, StatusCode};
use std::future::Future;
use std::sync::Arc;
use std::{collections::HashMap, pin::Pin};

pub(crate) type RouteFn = Box<dyn Fn(Request, Params) -> AsyncResponse + Send + Sync>;

#[derive(Debug)]
pub struct StaticSegment {
    pub value: &'static str,
    pub position: usize,
}

#[derive(Debug)]
pub struct DynamicSegment {
    pub name: &'static str,
    pub position: usize,
}

pub struct Route {
    pub static_segments: Vec<StaticSegment>,
    pub dynamic_segments: Vec<DynamicSegment>,
    pub handler: RouteFn,
}

pub struct Router {
    table: HashMap<Method, Vec<Route>>,
}

impl Router {
    pub fn new() -> Self {
        Router {
            table: HashMap::new(),
        }
    }

    pub fn add(&mut self, method: Method, route: impl Fn() -> Route) {
        self.table
            .entry(method)
            .or_insert(vec![route()])
            .push(route());
    }

    pub(crate) fn lookup(
        self: Arc<Self>,
        req: &mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<Response, std::io::Error>> + Send>> {
        let method = req.method();
        let raw_route = RawRoute::from_path(req.uri().path().into());
        let maybe_route = if let Some(routes) = self.table.get(method) {
            routes
                .iter()
                .filter(|route| paths_match(route, &raw_route))
                .nth(0)
        } else {
            return Box::pin(not_found());
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

            return Box::pin((route.handler)(req, params));
        }

        Box::pin(not_found())
    }
}

fn paths_match(route: &Route, raw_route: &RawRoute) -> bool {
    if raw_route.raw_segments.len() == route.static_segments.len() + route.dynamic_segments.len() {
        let static_matches = route
            .static_segments
            .iter()
            .fold(true, |is_match, static_segment| {
                let raw_segment = &raw_route.raw_segments[static_segment.position];
                is_match & (raw_segment == static_segment)
            });

        let dynamic_matches =
            route
                .dynamic_segments
                .iter()
                .fold(true, |is_match, dynamic_segment| {
                    let raw_segment = &raw_route.raw_segments[dynamic_segment.position];
                    is_match & (raw_segment == dynamic_segment)
                });

        static_matches & dynamic_matches
    } else {
        false
    }
}

async fn not_found() -> Result<Response, std::io::Error> {
    use crate::endpoint::error_response;
    use serde_json::json;

    error_response(json!("not found"), StatusCode::NotFound)
}

#[derive(Debug)]
pub(crate) struct RawSegment<'s> {
    value: &'s str,
    position: usize,
}

#[derive(Debug)]
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
