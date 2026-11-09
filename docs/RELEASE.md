# SDK and application release

Version 0.1.0 is a local release candidate. Package implementation can be validated
without publishing registries or deploying a domain. Public acceptance remains
separate from local tests; the checklist below records those boundaries.

## Build and verify locally

Use the locked dependencies with Rust stable, Node 24+, npm, Python 3.11+ and
OpenSSL supporting Ed25519. Run `npm ci` and `cargo fetch --locked` once, then:

```sh
python3 -m venv .local/release-venv
.local/release-venv/bin/pip install -r scripts/requirements-release.txt
.local/release-venv/bin/python scripts/validate-contract.py
python3 -m unittest discover -s scripts -p test_release.py
# Generate a LOCAL VALIDATION key only; production signing keys need separate custody.
openssl genpkey -algorithm ED25519 -out .local/release-validation.key
chmod 600 .local/release-validation.key
python3 scripts/release.py build --allow-dirty --output .local/releases/0.1.0-local --signing-key .local/release-validation.key
python3 scripts/release.py verify .local/releases/0.1.0-local --public-key .local/releases/0.1.0-local/release-public.pem
```

A local validation key proves the signing path, not publisher identity. For an
actual release, use a clean tagged checkout, omit `--allow-dirty`, and obtain the
trusted public key through an independent channel. Verification requires that
explicit key and rejects changed artifacts/manifests, swapped keys, duplicate or
unsafe artifact paths and a checksum list inconsistent with the signed manifest.
Protect production private keys outside the checkout; do not reuse validation keys.

The builder refuses existing output directories and version mismatches. It runs
TypeScript checking/build, SDK wire checks, Cargo package verification, then packs
and installs the actual npm tarball in a temporary consumer to compile its types
and exercise its exported API. Cargo compiles the packaged crate independently.
Packages include MIT/license notices and no operator credentials/databases/MMDB.

Four artifacts are generated:

- `vuuut-click-0.1.0.tgz`: `@vuuut/click` npm module, TypeScript declarations,
  server API subpath and standalone tracker/vitals scripts.
- `click-sdk-0.1.0.crate`: Rust SDK source with normalized Cargo metadata and license.
- `vuuut-click-browser-0.1.0.tar.gz`: versioned scripts/declarations and notices.
- `vuuut-click-operator-0.1.0.tar.gz`: docs, deployment/monitoring configuration,
  migrations, lockfile and operational scripts. The backend image is built by CI
  from source; this operator bundle is not a standalone server binary.

SHA256SUMS and release-manifest.json bind artifact sizes/hashes, source commit and
tree digest, dirty status, migration hashes and SHA384 script SRI. Optional local
signing adds release-manifest.sig/release-public.pem. The builder always records
`production_accepted:false`; a valid package is not a production acceptance claim.
Generated candidate artifacts stay ignored under `.local`.

## Remote CI and publication

The repository is https://github.com/anisulkibria/vuuut-click. The owner commits and
pushes changes. No commit/tag/push or registry publication is implicit in local
packaging. The latest inspected Priority 5 run failed at Playwright's Chromium
installation on the moving ubuntu-latest runner; its security/image job passed.
Priority 6 pins Ubuntu 24.04, fixes the Rust toolchain components input, updates
Node-based actions and adds contract/package/signature checks. This correction
must pass remotely on the final committed revision before release.

After committing/pushing and obtaining green verify CI, create/push the matching
version tag through the owner's normal workflow. Dispatch release-sdk on that
tag ref (for example `gh workflow run release.yml --ref v0.1.0 -f tag=v0.1.0`). It requires the latest push CI for the exact tag commit to pass, builds a
clean candidate and creates a draft GitHub release. GitHub's OIDC attestation
signs provenance for each artifact and manifest. Consumers verify identity and
source revision, for example:

```sh
gh attestation verify vuuut-click-0.1.0.tgz \
  --repo anisulkibria/vuuut-click \
  --signer-workflow anisulkibria/vuuut-click/.github/workflows/release.yml \
  --source-ref refs/tags/v0.1.0 \
  --source-digest EXPECTED_COMMIT
```

Follow [GitHub's attestation verification documentation](https://cli.github.com/manual/gh_attestation_verify)
for offline bundles and trust policies. A successful remote attestation is not
claimed before that workflow runs.

Registry publication is an explicit workflow option behind the `sdk-release`
environment. Configure that environment's reviewer policy first, confirm ownership
of `@vuuut/click` and `click-sdk`, and configure npm trusted publishing for the
repository/workflow/environment. The npm manifest uses the actual repository URL.
See [npm's trusted publisher setup](https://docs.npmjs.com/trusted-publishers/).
The Rust registry needs a narrowly scoped `CARGO_REGISTRY_TOKEN` environment secret.
No registry credentials or ownership were supplied for this local checkpoint.

Publishing verifies GitHub-attested downloads first, then publishes the npm
artifact with provenance and the same-tag Rust SDK. Registries have independent
transactions: if one publish succeeds and the other fails, preserve the successful
version and rerun only the failed registry job after checking the registry; npm
and Rust publish as separate jobs.
Never silently overwrite/relabel a published version. Publish the draft release
only after checking its attachments, provenance and release notes.

## Deployment acceptance

| Gate | Evidence/status |
| --- | --- |
| Backend/store and browser behavior | Priority 5: 51 backend checks and 11 browser scenarios passed |
| SDK/contract/signing | 85 operations/46 schemas, 28 live responses, 11 Python guards, installed npm types/wire checks and Cargo package verification passed locally |
| Recovery/monitoring/Linux/security | Priority 5 local evidence; operations runbook records limits |
| Remote final-revision CI | Pending owner commit/push and successful run |
| Registry ownership/publication/provenance | Prepared workflow; accounts/trust configuration pending |
| Public domain/TLS/licensed GeoIP/sender alignment | Explicitly deferred by owner |
| Off-machine backup/escrow/full-volume soak/independent review | External deployment gates outstanding |

Before promotion, record the deployed image digest, exact source/migration hashes,
remote CI and artifact identities, licensed MMDB version, backup restore evidence,
capacity limits, alert routing, independent review and public end-to-end collection/
billing/mail/native results. Test Stripe validates no live commercial configuration.
Refer to MIGRATIONS.md for upgrade/rollback and ADMIN_GUIDE.md for routine operation.

Local verification on 2026-11-10 also passed strict Clippy, formatting, actionlint
and a clean source/license/secret scan. Four candidate artifacts were signed and
verified using an ignored local validation key. Original-account Events/login and
TEST Stripe/SMTP smoke passed. These checks do not replace remote CI or public
registry evidence. The unchanged tracker bundles retain Priority 5 browser/store
coverage; those full suites were not repeated for packaging/documentation changes.
