//! Enforcement of the process-wide [`egress_guard`] policy for every request sent through
//! [`crate::Client`].

use std::sync::Arc;

/// Reserved (RFC 2606) suffix that never resolves. `reqwest::Error` cannot be constructed outside
/// reqwest, so a refused request is re-pointed at `<original host>.<this suffix>`, which
/// [`GuardedResolver`] always refuses. Callers then see an ordinary connect error whose source is
/// [`egress_guard::Blocked`], and no byte ever leaves the machine.
const BLOCKED_HOST_SUFFIX: &str = "egress-blocked.invalid";

const MAX_REDIRECTS: usize = 10;

/// Applies a restricted egress policy to a client under construction: DNS resolution and
/// redirects are checked against the allowlist, and proxies are bypassed so the checked host is
/// always the host actually connected to. Unrestricted builds keep reqwest's defaults; the policy
/// is switched on before any client is created.
pub(crate) fn guard_client_builder(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    if !egress_guard::is_restricted() {
        return builder;
    }
    builder
        .dns_resolver(Arc::new(GuardedResolver))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                attempt.error("too many redirects")
            } else if let Err(blocked) = egress_guard::check_url(attempt.url()) {
                attempt.error(blocked)
            } else {
                attempt.follow()
            }
        }))
        .no_proxy()
}

/// Re-points `url` at an unresolvable host when the policy refuses it.
pub(crate) fn redirect_if_blocked(url: &mut reqwest::Url) {
    if egress_guard::check_url(url).is_ok() {
        return;
    }
    let label = match url.host() {
        Some(url::Host::Domain(domain)) => domain.to_owned(),
        Some(url::Host::Ipv4(ip)) => ip.to_string(),
        Some(url::Host::Ipv6(_)) | None => "request".to_owned(),
    };
    *url = reqwest::Url::parse(&format!("http://{label}.{BLOCKED_HOST_SUFFIX}/")).unwrap_or_else(
        |_| {
            reqwest::Url::parse(&format!("http://request.{BLOCKED_HOST_SUFFIX}/"))
                .expect("static URL is valid")
        },
    );
}

/// Same as [`redirect_if_blocked`] for requests that are sent through `reqwest_eventsource`, which
/// only accepts a `reqwest::RequestBuilder`.
pub(crate) fn guard_request_builder(builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    if !egress_guard::is_restricted() {
        return builder;
    }
    // Event-source requests must be cloneable, so a probe can be built without consuming the
    // caller's builder.
    let Some(probe) = builder.try_clone() else {
        return builder;
    };
    match probe.build_split() {
        (client, Ok(mut request)) if !egress_guard::is_url_allowed(request.url()) => {
            redirect_if_blocked(request.url_mut());
            reqwest::RequestBuilder::from_parts(client, request)
        }
        _ => builder,
    }
}

struct GuardedResolver;

impl reqwest::dns::Resolve for GuardedResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(resolve_guarded(name.as_str().to_owned()))
    }
}

type BoxError = Box<dyn std::error::Error + Send + Sync>;

async fn resolve_guarded(host: String) -> Result<reqwest::dns::Addrs, BoxError> {
    if let Some(original) = host.strip_suffix(&format!(".{BLOCKED_HOST_SUFFIX}")) {
        // Already recorded when the URL was rewritten.
        return Err(Box::new(egress_guard::Blocked {
            target: original.to_owned(),
        }));
    }
    egress_guard::check_host(&host)?;
    let addrs: Vec<_> = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
    Ok(Box::new(addrs.into_iter()))
}
