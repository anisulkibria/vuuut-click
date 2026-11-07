//! Public, Rust-served marketing page. No authentication or database lookup is needed.
use crate::dashboard::document;
use axum::{
    http::{header, HeaderMap},
    response::{Html, IntoResponse},
};

pub async fn home(headers: HeaderMap) -> Html<String> {
    let Html(page) = document(
        "A little insight. A world of possibility.",
        &headers,
        include_str!("../../../dashboard/landing/index.html"),
    );
    Html(page.replacen("</head>",concat!(
        "<meta name=\"description\" content=\"Get to know the people finding your corner of the internet. Thoughtful website analytics for traffic, pages, sources, countries, and custom goals.\">",
        "<link rel=\"stylesheet\" href=\"/assets/landing.css\"></head>"
    ),1))
}

pub async fn stylesheet() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=300"),
        ],
        include_str!("../../../dashboard/landing/landing.css"),
    )
}
