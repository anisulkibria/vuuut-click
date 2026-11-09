/** Server-side API client. Never expose an integration credential in browser code. */
export type Period = 1 | 7 | 30 | 90;
export type Breakdown = {
    label: string;
    count: number;
};
export type Overview = {
    pageviews: number;
    visitors: number;
    sessions: number;
    realtime: number;
    bounce_rate: number;
    timeseries: Breakdown[];
};
export type FunnelStep = {
    kind: 'page' | 'page_prefix' | 'event';
    value: string;
};
export type Funnel = {
    id: string;
    name: string;
    window_seconds: number;
    scope: 'session' | 'visitor';
    steps: FunnelStep[];
};
export type FunnelReport = Omit<Funnel, 'steps'> & {
    steps: (FunnelStep & {
        sessions: number;
        participants: number;
        conversion_rate: number;
        drop_off: number;
    })[];
};
export type Journey = {
    session_id: string;
    country: string;
    events: number;
    steps: [number, string, string][];
};
export type Vital = {
    metric: string;
    samples: number;
    p50: number;
    p75: number;
    p95: number;
};
export type Revenue = {
    label: string;
    currency: string;
    total_minor: string;
    payments: number;
    dimension_present: 0 | 1;
};
export type BackendEvent = {
    external_id: string;
    kind: 'payment' | 'refund' | 'member_signup' | 'subscription_started' | 'subscription_cancelled' | 'newsletter_sent' | 'newsletter_open' | 'newsletter_click';
    session_id?: string;
    visitor_id?: string;
    path?: string;
    amount_minor?: number;
    currency?: string;
    occurred_at_ms?: number;
    newsletter_id?: string;
    recipient_id?: string;
};
export declare class ClickApi {
    private key;
    private base;
    constructor(base: string, key: string);
    private request;
    overview(site: string, days?: Period): Promise<Overview>;
    breakdown(site: string, dimension: 'countries' | 'pages' | 'referrers' | 'campaigns' | 'devices' | 'browsers' | 'events' | 'operating_systems', days?: Period): Promise<Breakdown[]>;
    funnels(site: string): Promise<Funnel[]>;
    funnel(site: string, id: string, days?: Period): Promise<FunnelReport>;
    journeys(site: string, days?: Period): Promise<Journey[]>;
    vitals(site: string, days?: Period): Promise<Vital[]>;
    revenue(site: string, days?: Period, model?: 'first_touch' | 'last_touch' | 'last_non_direct', dimension?: 'source' | 'campaign' | 'landing' | 'medium' | 'channel' | 'country' | 'device'): Promise<Revenue[]>;
    comparison(site: string, days?: Period): Promise<Comparison>;
    content(site: string, days?: Period): Promise<ContentMetrics[]>;
    acquisition(site: string, days?: Period, dimension?: 'source' | 'medium' | 'campaign' | 'landing' | 'channel'): Promise<AcquisitionMetrics[]>;
    retention(site: string, days?: Period): Promise<Retention>;
    vitalComparison(site: string, days?: Period): Promise<VitalComparison>;
    newsletters(site: string, days?: Period): Promise<NewsletterReport>;
    campaignReturns(site: string, days?: Period, model?: 'first_touch' | 'last_touch' | 'last_non_direct'): Promise<CampaignReturn[]>;
    recordCost(site: string, input: CostInput): Promise<{
        id: string;
        revision: number;
    }>;
    events(site: string, days?: Period, filter?: {
        event?: string;
        path?: string;
        campaign?: string;
    }): Promise<EventDetail[]>;
    backendEvent(site: string, event: BackendEvent): Promise<{
        event_id: string;
        accepted: number;
        duplicates: number;
    }>;
}
export type Snapshot = {
    pageviews: number;
    visitors: number;
    sessions: number;
    bounced_sessions: number;
    visible_ms: number;
    measured_sessions: number;
};
export type Comparison = {
    start: number;
    end: number;
    previous_start: number;
    previous_end: number;
    current: Snapshot;
    previous: Snapshot;
};
export type ContentMetrics = {
    path: string;
    pageviews: number;
    visitors: number;
    sessions: number;
    entries: number;
    exits: number;
    converting_sessions: number;
    visible_ms: number;
    measured_sessions: number;
    mean_scroll_percent: number | null;
};
export type AcquisitionMetrics = {
    label: string;
    sessions: number;
    pageviews: number;
    visitors: number;
    converting_sessions: number;
};
export type Retention = {
    history_start: number;
    identity: {
        new_visitors: number;
        returning_visitors: number;
        session_only_sessions: number;
        unclassified_sessions: number;
    };
    cohorts: {
        week: string;
        visitors: number;
        retained: (number | null)[];
    }[];
};
export type VitalComparison = {
    current: Vital[];
    previous: Vital[];
    start: number;
    end: number;
    previous_start: number;
    previous_end: number;
};
export type NewsletterReport = {
    editions: {
        newsletter_id: string;
        sent_recipients: number;
        opened_recipients: number;
        clicked_recipients: number;
        link_click_facts: number;
    }[];
    unassigned_events: number;
    activity_outside_send_cohort: number;
};
export type CostInput = {
    reference: string;
    revision: number;
    day: string;
    campaign: string;
    currency: string;
    amount_minor: number;
    source: string;
};
export type CampaignReturn = {
    campaign: string;
    currency: string;
    net_revenue_minor: string;
    revenue_recorded: boolean;
    attribution_complete: boolean;
    cost_minor: string | null;
    covered_days: number;
    expected_days: number;
    sources: string[];
    roi_percent: number | null;
    roas: number | null;
};
export type EventDetail = {
    event: string;
    path: string;
    campaign: string;
    count: number;
    sessions: number;
    visitors: number;
};
