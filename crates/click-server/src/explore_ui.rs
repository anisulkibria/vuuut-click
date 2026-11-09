//! Native HTML tables for the extended metric contract.
use crate::{analytics, dashboard::esc, explore, AppResult};
use click_store::{Store, Website};
fn panel(title: &str, headers: &[&str], rows: &str, note: &str) -> String {
    panel_with_controls(title, headers, rows, note, "")
}
fn panel_with_controls(
    title: &str,
    headers: &[&str],
    rows: &str,
    note: &str,
    controls: &str,
) -> String {
    let head = headers
        .iter()
        .map(|h| format!("<th scope=\"col\">{}</th>", esc(h)))
        .collect::<String>();
    let empty = format!(
        "<tr><td colspan=\"{}\">No recorded measurements in this period.</td></tr>",
        headers.len()
    );
    let region = if controls.is_empty() {
        String::new()
    } else {
        format!(
            " role=\"region\" aria-label=\"{}\" tabindex=\"0\"",
            esc(title)
        )
    };
    format!("<section class=\"panel\"><div class=\"panel-heading\"><h2>{}</h2></div>{controls}<div class=\"owner-table\"{region}><table><thead><tr>{head}</tr></thead><tbody>{}</tbody></table></div><p class=\"muted\">{}</p></section>",esc(title),if rows.is_empty(){&empty}else{rows},esc(note))
}
fn cell(value: impl std::fmt::Display) -> String {
    format!("<td>{}</td>", esc(&value.to_string()))
}
fn number(value: Option<f64>, suffix: &str) -> String {
    value
        .map(|n| format!("{n:.1}{suffix}"))
        .unwrap_or_else(|| "Unavailable".into())
}
fn time(value: u32) -> String {
    chrono::DateTime::from_timestamp(value as i64, 0)
        .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_default()
}
pub async fn comparison(store: &Store, site: &Website, start: u32, end: u32) -> AppResult<String> {
    let r = explore::comparison_data(store, site, start, end).await?;
    let c = &r.current;
    let p = &r.previous;
    let mut rows = String::new();
    for (label, current, previous) in [
        ("Page views", c.pageviews, p.pageviews),
        ("Visitors", c.visitors, p.visitors),
        ("Sessions", c.sessions, p.sessions),
    ] {
        let change =
            (previous > 0).then(|| (current as f64 - previous as f64) / previous as f64 * 100.0);
        rows.push_str(&format!(
            "<tr><th scope=\"row\">{label}</th>{}{}{}</tr>",
            cell(current),
            cell(previous),
            cell(number(change, "%"))
        ));
    }
    rows.push_str(&format!(
        "<tr><th scope=\"row\">Bounce rate</th>{}{}{}</tr>",
        cell(number((c.sessions > 0).then(|| c.bounce()), "%")),
        cell(number((p.sessions > 0).then(|| p.bounce()), "%")),
        cell(number(
            (c.sessions > 0 && p.sessions > 0).then(|| c.bounce() - p.bounce()),
            " percentage points"
        ))
    ));
    let change = c
        .visible_seconds()
        .zip(p.visible_seconds())
        .and_then(|(c, p)| (p > 0.0).then_some((c - p) / p * 100.0));
    rows.push_str(&format!(
        "<tr><th scope=\"row\">Visible seconds / measured session</th>{}{}{}</tr>",
        cell(number(c.visible_seconds(), " s")),
        cell(number(p.visible_seconds(), " s")),
        cell(number(change, "%"))
    ));
    let note=format!("Current: {} to {}. Previous: {} to {}. Equal-duration windows. A zero baseline has no relative change. Engagement covers {} of {} current sessions and requires optional data-engagement=\"true\" tracking.",time(start),time(end),time(r.previous_start),time(r.previous_end),c.measured_sessions,c.sessions);
    Ok(panel(
        "Compared with the previous period",
        &["Metric", "Current", "Previous", "Change"],
        &rows,
        &note,
    ))
}
pub async fn content(store: &Store, site: &Website, start: u32, end: u32) -> AppResult<String> {
    let data = explore::content_data(store, site, start, end).await?;
    let mut rows = String::new();
    for r in data {
        let rate = if r.sessions == 0 {
            None
        } else {
            Some(r.converting_sessions as f64 / r.sessions as f64 * 100.0)
        };
        let seconds = (r.measured_sessions > 0)
            .then(|| r.visible_ms as f64 / 1000.0 / r.measured_sessions as f64);
        rows.push_str(&format!(
            "<tr><th scope=\"row\">{}</th>{}{}{}{}{}{}{}{}{}</tr>",
            esc(&r.path),
            cell(r.pageviews),
            cell(r.sessions),
            cell(r.entries),
            cell(r.exits),
            cell(r.converting_sessions),
            cell(number(rate, "%")),
            cell(number(seconds, " s")),
            cell(r.measured_sessions),
            cell(number(r.mean_scroll_percent, "%"))
        ));
    }
    Ok(panel("Entry, exit and content engagement",&["Page","Views","Sessions","Entries","Exits","Goal sessions","Goal rate","Visible seconds / measured session","Measured sessions","Mean maximum scroll"],&rows,"Top 100 paths by pageviews. Entry and exit use each session's first/last pageview inside this period. Goal sessions also viewed this path; this is association, not causal attribution. Visible time and scroll require optional telemetry; missing coverage is unavailable."))
}
pub async fn acquisition(
    store: &Store,
    site: &Website,
    start: u32,
    end: u32,
    dimension: &str,
    days: u32,
) -> AppResult<String> {
    let data = explore::acquisition_data(store, site, start, end, dimension).await?;
    let mut rows = String::new();
    let mut options = String::new();
    for (key, label) in [
        ("channel", "Channel"),
        ("source", "Source"),
        ("medium", "Medium"),
        ("campaign", "Campaign"),
        ("landing", "Landing page"),
    ] {
        options.push_str(&format!(
            "<option value=\"{key}\" {}>{label}</option>",
            if key == dimension { "selected" } else { "" }
        ));
    }
    for r in data {
        let rate =
            (r.sessions > 0).then(|| r.converting_sessions as f64 / r.sessions as f64 * 100.0);
        rows.push_str(&format!(
            "<tr><th scope=\"row\">{}</th>{}{}{}{}{}</tr>",
            esc(&r.label),
            cell(r.sessions),
            cell(r.pageviews),
            cell(r.visitors),
            cell(r.converting_sessions),
            cell(number(rate, "%"))
        ));
    }
    let form=format!("<form class=\"settings-switch\" method=\"get\" action=\"/dashboard\"><input type=\"hidden\" name=\"site\" value=\"{}\"><input type=\"hidden\" name=\"view\" value=\"acquisition\"><input type=\"hidden\" name=\"days\" value=\"{days}\"><label>Group sessions by<select name=\"dimension\">{options}</select></label><button class=\"btn outlined small\">Apply grouping</button></form>",site.id);
    Ok(form+&panel("Session acquisition",&["Entry attribution","Sessions","Page views","Entry visitor IDs","Goal sessions","Goal rate"],&rows,"Top 100 groups. A session belongs to its first observed pageview's source, medium, campaign and landing page inside this period. Channels use the documented medium/source rules. Goal rate counts distinct goal-converting sessions."))
}
pub async fn audience(
    store: &Store,
    site: &Website,
    start: u32,
    end: u32,
    days: u32,
) -> AppResult<String> {
    let r = explore::retention_data(store, site, start, end).await?;
    let i = r.identity;
    let rows=format!("<tr><th scope=\"row\">New provided visitor IDs</th>{}</tr><tr><th scope=\"row\">Returning provided visitor IDs</th>{}</tr><tr><th scope=\"row\">Session-only sessions</th>{}</tr><tr><th scope=\"row\">Unclassified sessions</th>{}</tr>",cell(i.new_visitors),cell(i.returning_visitors),cell(i.session_only_sessions),cell(i.unclassified_sessions));
    Ok(panel("New and returning identifiers",&["Identity coverage","Count"],&rows,&format!("First observed in retained history since {}. Session-only and unclassified data cannot identify returning browsers. These are identifiers, not people.",time(r.history_start)))+&format!("<p><a class=\"btn outlined\" href=\"/insights?site={}&amp;view=retention&amp;days={days}\">View retention cohorts →</a></p>",site.id))
}
pub async fn retention(store: &Store, site: &Website, start: u32, end: u32) -> AppResult<String> {
    let data = explore::retention_data(store, site, start, end).await?;
    let mut rows = String::new();
    for c in data.cohorts {
        rows.push_str(&format!(
            "<tr><th scope=\"row\">{}</th>{}",
            esc(&c.week),
            cell(c.visitors)
        ));
        for count in c.retained {
            match count {
                Some(n) => {
                    let percent = if c.visitors == 0 {
                        0.0
                    } else {
                        n as f64 / c.visitors as f64 * 100.0
                    };
                    rows.push_str(&format!("<td style=\"background:color-mix(in srgb,var(--primary) {:.0}%,transparent)\">{percent:.1}% ({n}/{})</td>",percent*0.25,c.visitors));
                }
                None => rows.push_str("<td>Not complete</td>"),
            }
        }
        rows.push_str("</tr>");
    }
    Ok(panel("Weekly visitor retention",&["First observed week (UTC)","Provided visitor IDs","Week 0","Week 1","Week 2","Week 3","Week 4"],&rows,&format!("Only explicitly supplied visitor IDs with pageviews qualify. First observed means within retained history since {}, not a lifetime first visit. New IDs in the selected period form each cohort. Week 0 is observed membership; later incomplete weeks remain unavailable.",time(data.history_start))))
}
pub async fn vitals(store: &Store, site: &Website, start: u32, end: u32) -> AppResult<String> {
    let r = explore::vital_comparison_data(store, site, start, end).await?;
    let mut rows = String::new();
    let names: std::collections::BTreeSet<_> = r
        .current
        .iter()
        .chain(r.previous.iter())
        .map(|m| m.metric.clone())
        .collect();
    for name in names {
        let c = r.current.iter().find(|m| m.metric == name);
        let p = r.previous.iter().find(|m| m.metric == name);
        let unit = if name == "CLS" { "" } else { " ms" };
        rows.push_str(&format!(
            "<tr><th scope=\"row\">{}</th>{}{}{}{}{}</tr>",
            esc(&name),
            cell(number(c.map(|m| m.p75), unit)),
            cell(c.map(|m| m.samples).unwrap_or(0)),
            cell(number(p.map(|m| m.p75), unit)),
            cell(p.map(|m| m.samples).unwrap_or(0)),
            cell(number(c.zip(p).map(|(c, p)| c.p75 - p.p75), unit))
        ));
    }
    Ok(panel("Web Vitals period comparison",&["Metric","Current p75","Current samples","Previous p75","Previous samples","p75 change"],&rows,&format!("Previous equal-duration window: {} to {}. Lower values indicate faster or more stable experiences. Missing measurements remain unavailable; sample sizes can differ.",time(r.previous_start),time(r.previous_end))))
}

pub async fn newsletters(store: &Store, site: &Website, start: u32, end: u32) -> AppResult<String> {
    let data = crate::newsletters::data(store, site, start, end).await?;
    let mut rows = String::new();
    for r in data.editions {
        let opened = (r.sent_recipients > 0)
            .then(|| r.opened_recipients as f64 / r.sent_recipients as f64 * 100.0);
        let clicked = (r.sent_recipients > 0)
            .then(|| r.clicked_recipients as f64 / r.sent_recipients as f64 * 100.0);
        let label = format!(
            "<code title=\"{}\">{}</code>",
            esc(&r.newsletter_id),
            esc(&r.newsletter_id.chars().take(12).collect::<String>())
        );
        rows.push_str(&format!(
            "<tr><th scope=\"row\">{label}</th>{}{}{}{}{}{}</tr>",
            cell(r.sent_recipients),
            cell(r.opened_recipients),
            cell(r.clicked_recipients),
            cell(r.link_click_facts),
            cell(number(opened, "%")),
            cell(number(clicked, "%"))
        ));
    }
    Ok(panel("Newsletter send cohorts",&["Edition key","Sent recipients","Opened recipients","Clicked recipients","Link / recipient facts","Open rate","Click rate"],&rows,&format!("Top 100 editions by sends captured in this period; activity is matched through its end using source occurrence time after send. {} facts lack edition/recipient grouping; {} open/click facts fall outside the captured send cohort. No rates are inferred for those facts. Edition keys are opaque; the CMS resolves titles locally.",data.unassigned_events,data.activity_outside_send_cohort)))
}
pub async fn campaigns(
    store: &Store,
    site: &Website,
    start: u32,
    end: u32,
    model: &str,
    days: u32,
    manage: bool,
) -> AppResult<String> {
    let data = crate::costs::returns_data(store, site, start, end, model).await?;
    let mut rows = String::new();
    for r in data {
        let revenue = if r.revenue_recorded {
            analytics::money(&r.net_revenue_minor, &r.currency)?
        } else {
            "No revenue recorded".into()
        };
        let cost = r
            .cost_minor
            .as_deref()
            .map(|v| analytics::money(v, &r.currency))
            .transpose()?
            .unwrap_or_else(|| "No cost recorded".into());
        rows.push_str(&format!(
            "<tr><th scope=\"row\">{}</th>{}{}{}{}{}{}{}</tr>",
            esc(&r.campaign),
            cell(&r.currency),
            cell(revenue),
            cell(cost),
            cell(format!("{} / {} days", r.covered_days, r.expected_days)),
            cell(number(r.roi_percent, "%")),
            cell(number(r.roas, "×")),
            cell(r.sources.join(", "))
        ));
    }
    let mut options = String::new();
    for (key, label) in [
        ("first_touch", "First touch"),
        ("last_touch", "Last touch"),
        ("last_non_direct", "Last non-direct"),
    ] {
        options.push_str(&format!(
            "<option value=\"{key}\" {}>{label}</option>",
            if key == model { "selected" } else { "" }
        ));
    }
    let mut html=format!("<form class=\"settings-switch\" method=\"get\" action=\"/insights\"><input type=\"hidden\" name=\"site\" value=\"{}\"><input type=\"hidden\" name=\"view\" value=\"campaigns\"><input type=\"hidden\" name=\"days\" value=\"{days}\"><label>Revenue attribution<select name=\"model\">{options}</select></label><button class=\"btn outlined small\">Apply attribution</button></form>",site.id);
    html.push_str(&panel("Campaign costs and returns",&["Campaign","Currency","Net revenue","Recorded cost","Cost coverage","ROI","ROAS","Cost sources"],&rows,"ROI requires complete campaign attribution, observed same-currency revenue, positive cost and explicit records for every selected UTC date. Missing, incomplete and zero cost yield unavailable ROI. Costs are supplied records with source provenance; Click has not independently verified an advertising provider. No currency conversion is inferred. The current UTC date is incomplete and its records may change."));
    if manage && !site.is_demo {
        let currencies = [
            "USD", "EUR", "GBP", "CAD", "AUD", "NZD", "SGD", "INR", "BDT", "JPY",
        ]
        .iter()
        .map(|c| format!("<option>{c}</option>"))
        .collect::<String>();
        html.push_str(&format!(r#"<section class="panel"><div class="panel-heading"><h2>Record an advertising cost</h2></div><form class="stack" action="/insights/cost" method="post"><input type="hidden" name="site" value="{}"><input type="hidden" name="days" value="{days}"><label>Campaign label<input name="campaign" maxlength="120" required placeholder="Match your UTM campaign label"></label><label>UTC cost date<input type="date" name="day" value="{}" max="{}" required></label><label>Currency<select name="currency">{currencies}</select></label><label>Amount<input name="amount" type="number" min="0" step="0.01" required placeholder="5.00"></label><label>Source<input name="source" maxlength="32" pattern="[a-z0-9_-]+" required placeholder="advertising_ledger"></label><label>Source record reference<input name="reference" maxlength="200" required placeholder="Stable row or invoice reference"></label><label>Record version<input type="number" name="revision" min="1" value="1" required></label><p class="muted">Use amounts from your advertising records. A correction uses the same source/reference and a higher version; retries reuse the same version. Explicit zero-cost dates establish coverage. JPY amounts must be whole numbers. Do not enter personal data or credentials.</p><button class="btn primary">Save cost record</button></form></section>"#,site.id,chrono::Utc::now().date_naive(),chrono::Utc::now().date_naive()));
    }
    Ok(html)
}

pub async fn events(
    store: &Store,
    site: &Website,
    start: u32,
    end: u32,
    filter: &explore::EventQuery,
    days: u32,
) -> AppResult<String> {
    let data = explore::events_data(store, site, start, end, filter).await?;
    let mut rows = String::new();
    for r in data {
        rows.push_str(&format!(
            "<tr><th scope=\"row\">{}</th>{}{}{}{}{}</tr>",
            esc(&r.event),
            cell(r.path),
            cell(if r.campaign.is_empty() {
                "No campaign"
            } else {
                &r.campaign
            }),
            cell(r.count),
            cell(r.sessions),
            cell(r.visitors)
        ));
    }
    let form=format!("<form class=\"event-filters\" action=\"/dashboard\" method=\"get\"><input type=\"hidden\" name=\"site\" value=\"{}\"><input type=\"hidden\" name=\"view\" value=\"events\"><input type=\"hidden\" name=\"days\" value=\"{days}\"><label>Filter event<input name=\"event\" maxlength=\"64\" value=\"{}\"></label><label>Filter path<input name=\"path\" maxlength=\"1024\" value=\"{}\"></label><label>Filter campaign<input name=\"campaign\" maxlength=\"120\" value=\"{}\"></label><button class=\"btn outlined small\">Filter events</button></form>",site.id,esc(filter.event.as_deref().unwrap_or("")),esc(filter.path.as_deref().unwrap_or("")),esc(filter.campaign.as_deref().unwrap_or("")));
    Ok(format!("<div class=\"event-context\">{}</div>", panel_with_controls("Event context and filters",&["Event","Path","Campaign","Completions","Sessions","Visitor IDs"],&rows,"Top 100 matching event/path/campaign groups. Blank event filters show custom/backend events, excluding pageviews and automatic telemetry. Only minimized path and campaign context is available; no form values or arbitrary query parameters are collected.", &form)))
}
