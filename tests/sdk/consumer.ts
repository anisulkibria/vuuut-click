import { createTracker } from '@vuuut/click';
import { ClickApi, type Overview, type BackendEvent } from '@vuuut/click/api';

// Compile the installed tarball's exports/declarations, not the source tree.
const tracker: ReturnType<typeof createTracker> = createTracker({
  endpoint: 'https://analytics.example.com/v1/collect/public-id',
  consent: 'required',
});
tracker.consent(true);
tracker.track('signup');
tracker.destroy();
const api = new ClickApi('https://analytics.example.com', 'server-only-key');
const report: Promise<Overview> = api.overview('website-id', 7);
const event: BackendEvent = {external_id:'stable-reference',kind:'payment',amount_minor:100,currency:'USD'};
void api.backendEvent('website-id', event);
void report;
