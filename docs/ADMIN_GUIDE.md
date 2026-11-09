# Customer and platform administration

Customers use `/signin`; platform owners use `/owner/signin`. Both can remain
signed in in the same browser. The public homepage stays accessible. Registering a
workspace never grants platform-owner access.

## Customer workspace

Choose a website in Dashboard. Overview shows traffic; the sidebar opens content,
acquisition, events/goals and audience reports. Advanced analytics includes funnels,
journeys, revenue, retention, newsletters, Web Vitals and campaign costs. Demo data
has a visible banner and never consumes a real website's usage. Real websites
remain separate. Missing measurements are explicit; see ANALYTICS_METRICS.md.

Open Workspace & settings to switch/rename workspaces, invite team members and
manage roles. Owners handle billing and destructive workspace actions; admins
manage websites/team within role constraints; analysts edit goals/funnels; viewers
read reports. The last owner cannot be removed. Invitations expire and are one-use;
a queued message does not prove inbox delivery.

Website setup requires an exact origin and verified email, then DNS ownership for
public sites. Install the supplied public tracker snippet and connect its consent
control before collection. Use a read credential only on servers that need reports;
use a separate integration credential for authoritative backend facts. Rotation
returns a secret once; update its consumers and revoke superseded credentials.

Billing & usage exposes actual accepted-event usage and Stripe-hosted checkout or
billing portal. Changes take effect after verified webhooks. Test billing is visibly
labeled. Use the portal for invoices, interval changes and cancellation. A failed
return page does not justify repeating payment with a new identity.

Privacy settings request bounded exports and durable deletion. Watch job status;
downloads expire. Retention changes apply only within the plan limit. Deletion is
irreversible; the confirmation names the website/workspace. Account settings manage
profile/password, email verification and other sessions.

## Platform owner

An operator grants an existing account with `click-server grant-owner <email>`.
Sign in separately, then open Account & security to configure MFA and save recovery
codes privately. Email recovery keeps MFA enabled. Do not promote an account merely
to access a customer's analytics.

Owner sidebar pages list actual workspaces, users, websites, subscriptions, event
delivery and audit entries. Suspension stops affected collection/access; reactivate
only after resolving its cause. Manual grants cannot override provider-linked
subscriptions. Event delivery distinguishes pending and dead-letter work. Fix the
underlying failure before retrying; retries preserve event identities and usage.
Commercial fulfillment shows bounded billing/mail status and eligible audited mail
retries. It does not expose payment keys or private message bodies.

Operational monitoring and backup tools are private operator functions, separate
from owner cookies. Follow OPERATIONS.md for dependency failures, alert handling,
credential rotation and recovery, and MIGRATIONS.md before installing upgrades.
