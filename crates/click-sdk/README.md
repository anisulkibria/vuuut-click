# click-sdk

Typed server-side client for Vuuut Click analytics, authoritative backend events
and the native CMS integration. Credentials must never be sent to a browser.

```rust,no_run
use click_sdk::Client;
use uuid::Uuid;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let client = Client::new("https://analytics.example.com", std::env::var("CLICK_READ_KEY")?)?;
let report = client.overview(Uuid::parse_str("00000000-0000-0000-0000-000000000001")?, 7).await?;
println!("{} pageviews", report.pageviews);
# Ok(())
# }
```

For a downloaded and signature-verified `.crate`, extract it and use a Cargo
path dependency, or configure your private registry. Public registry publication
requires the operator to confirm package-name ownership. The SDK uses Rustls,
rejects redirects and bounds HTTP calls to 15 seconds. Callers decide retry policy;
reuse backend event IDs and native provisioning revisions on uncertain outcomes.

`NativeTenant` and `Client::provision_native` synchronize managed sites;
`Client::connect_native` and `Client::exchange_native` handle pinned-key CMS
registration and short-lived staff exchange. Consult docs/GHOST_INTEGRATION.md before enabling native integrations.
The shipped OpenAPI contract describes the complete API beyond these typed
convenience methods. Release checksums, signatures and verification instructions
are included in the operator release bundle.
