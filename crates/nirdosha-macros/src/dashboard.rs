//! `dashboard! { .. }` — RFC 0009 Track C, Phase 0. Generates a
//! `mount_<name>(router) -> router` function registering `GET
//! <path>.json` (structured widget data, an OpenAPI entry for free)
//! and `GET <path>` (a server-rendered HTML page — no client-side JS)
//! on the existing `nirdosha_rt::web::Router`. Widget functions are
//! called directly in the generated code, so a wrong name or
//! signature is an ordinary `rustc` error, not a runtime surprise.
//!
//! ```ignore
//! nirdosha_rt::dashboard! {
//!     mount: mount_sales_dashboard,
//!     path: "/dashboard",
//!     title: "Sales Overview",
//!     refresh_seconds: 300,
//!     widgets {
//!         Metric { label: "Revenue MTD", fn: stat_revenue_mtd, target: 1000000, alert_below: true },
//!         Chart  { label: "By Region", fn: chart_revenue_by_region, mark: bar },
//!     }
//! }
//! ```

use crate::util::expect_keyword;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::{braced, Ident, Lit, LitBool, LitInt, LitStr, Token};

enum Widget {
    Metric {
        label: LitStr,
        stat_fn: Ident,
        target: Option<Lit>,
        alert_below: bool,
    },
    Chart {
        label: LitStr,
        chart_fn: Ident,
        mark: Ident,
    },
}

impl Parse for Widget {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let kind: Ident = input.parse()?;
        let content;
        braced!(content in input);
        match kind.to_string().as_str() {
            "Metric" => {
                let (mut label, mut stat_fn, mut target, mut alert_below) = (None, None, None, false);
                while !content.is_empty() {
                    let key = Ident::parse_any(&content)?;
                    content.parse::<Token![:]>()?;
                    match key.to_string().as_str() {
                        "label" => label = Some(content.parse::<LitStr>()?),
                        "fn" => stat_fn = Some(content.parse::<Ident>()?),
                        "target" => target = Some(content.parse::<Lit>()?),
                        "alert_below" => alert_below = content.parse::<LitBool>()?.value,
                        other => {
                            return Err(syn::Error::new(
                                key.span(),
                                format!("unknown Metric key `{other}` — valid keys: label, fn, target, alert_below"),
                            ))
                        }
                    }
                    if content.peek(Token![,]) {
                        content.parse::<Token![,]>()?;
                    }
                }
                Ok(Widget::Metric {
                    label: label.ok_or_else(|| syn::Error::new(kind.span(), "Metric needs `label`"))?,
                    stat_fn: stat_fn.ok_or_else(|| syn::Error::new(kind.span(), "Metric needs `fn`"))?,
                    target,
                    alert_below,
                })
            }
            "Chart" => {
                let (mut label, mut chart_fn, mut mark) = (None, None, None);
                while !content.is_empty() {
                    let key = Ident::parse_any(&content)?;
                    content.parse::<Token![:]>()?;
                    match key.to_string().as_str() {
                        "label" => label = Some(content.parse::<LitStr>()?),
                        "fn" => chart_fn = Some(content.parse::<Ident>()?),
                        "mark" => mark = Some(content.parse::<Ident>()?),
                        other => {
                            return Err(syn::Error::new(
                                key.span(),
                                format!("unknown Chart key `{other}` — valid keys: label, fn, mark"),
                            ))
                        }
                    }
                    if content.peek(Token![,]) {
                        content.parse::<Token![,]>()?;
                    }
                }
                let mark = mark.ok_or_else(|| syn::Error::new(kind.span(), "Chart needs `mark`"))?;
                if mark != "bar" && mark != "line" {
                    return Err(syn::Error::new(
                        mark.span(),
                        "mark must be `bar` or `line` (RFC 0009 Track C Phase 0 supports these two)",
                    ));
                }
                Ok(Widget::Chart {
                    label: label.ok_or_else(|| syn::Error::new(kind.span(), "Chart needs `label`"))?,
                    chart_fn: chart_fn.ok_or_else(|| syn::Error::new(kind.span(), "Chart needs `fn`"))?,
                    mark,
                })
            }
            other => Err(syn::Error::new(
                kind.span(),
                format!("unknown widget kind `{other}` — valid kinds: Metric, Chart"),
            )),
        }
    }
}

struct DashboardInput {
    mount: Ident,
    path: LitStr,
    title: LitStr,
    refresh_seconds: Option<LitInt>,
    widgets: Vec<Widget>,
}

impl Parse for DashboardInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        expect_keyword(input, "mount")?;
        input.parse::<Token![:]>()?;
        let mount: Ident = input.parse()?;
        input.parse::<Token![,]>()?;

        expect_keyword(input, "path")?;
        input.parse::<Token![:]>()?;
        let path: LitStr = input.parse()?;
        input.parse::<Token![,]>()?;

        expect_keyword(input, "title")?;
        input.parse::<Token![:]>()?;
        let title: LitStr = input.parse()?;
        input.parse::<Token![,]>()?;

        let mut refresh_seconds = None;
        if input.peek(Ident) {
            let fork = input.fork();
            let maybe: Ident = fork.parse()?;
            if maybe == "refresh_seconds" {
                input.parse::<Ident>()?;
                input.parse::<Token![:]>()?;
                refresh_seconds = Some(input.parse::<LitInt>()?);
                input.parse::<Token![,]>()?;
            }
        }

        expect_keyword(input, "widgets")?;
        let content;
        braced!(content in input);
        let mut widgets = Vec::new();
        while !content.is_empty() {
            widgets.push(content.parse::<Widget>()?);
            if content.peek(Token![,]) {
                content.parse::<Token![,]>()?;
            }
        }
        if widgets.is_empty() {
            return Err(syn::Error::new(content.span(), "dashboard! needs at least one widget"));
        }
        let _ = input.parse::<Token![,]>();
        Ok(DashboardInput {
            mount,
            path,
            title,
            refresh_seconds,
            widgets,
        })
    }
}

pub fn expand(input: TokenStream) -> TokenStream {
    let parsed = match syn::parse::<DashboardInput>(input) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };
    expand_parsed(parsed).into()
}

fn expand_parsed(input: DashboardInput) -> TokenStream2 {
    let mount = &input.mount;
    let path = &input.path;
    let title = &input.title;
    let refresh = match &input.refresh_seconds {
        Some(n) => quote! { Some(#n) },
        None => quote! { None },
    };
    let json_path = format!("{}.json", input.path.value().trim_end_matches('/'));

    let widget_pushes = input.widgets.iter().map(|w| match w {
        Widget::Metric { label, stat_fn, target, alert_below } => {
            let target_tok = match target {
                Some(lit) => quote! { Some(#lit as f64) },
                None => quote! { None },
            };
            quote! {
                widgets.push(::nirdosha_rt::dashboard::metric_widget(
                    #label,
                    ::nirdosha_rt::dashboard::IntoMetricValue::into_metric_value(#stat_fn()),
                    #target_tok,
                    #alert_below,
                ));
            }
        }
        Widget::Chart { label, chart_fn, mark } => {
            let mark_str = mark.to_string();
            quote! {
                widgets.push(::nirdosha_rt::dashboard::chart_widget(
                    #label,
                    #mark_str,
                    ::nirdosha_rt::dashboard::IntoChartData::into_chart_data(#chart_fn()),
                ));
            }
        }
    });

    quote! {
        fn #mount(router: ::nirdosha_rt::Router) -> ::nirdosha_rt::Router {
            fn __nirdosha_dashboard_widgets() -> Vec<::serde_json::Value> {
                let mut widgets: Vec<::serde_json::Value> = Vec::new();
                #(#widget_pushes)*
                widgets
            }
            router
                .get(#json_path, concat!(#title, " (JSON)"), |_req, _params| {
                    let widgets = __nirdosha_dashboard_widgets();
                    ::nirdosha_rt::Response::json(200, &::serde_json::json!({ "title": #title, "widgets": widgets }))
                })
                .get(#path, #title, |_req, _params| {
                    let widgets = __nirdosha_dashboard_widgets();
                    ::nirdosha_rt::Response::html(200, ::nirdosha_rt::dashboard::render_dashboard_html(#title, #refresh, &widgets))
                })
        }
    }
}
