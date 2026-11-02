# deploy/secrets

Docker secret files. **Everything in this directory except this README is git-ignored.**
Generate them on the VPS with `deploy/scripts/generate-secrets.sh` (never commit, never copy
to chat or tickets). Files are mode `0400`; MySQL files are owned by uid 999, app files by uid 10001.

| File | Used by | Content |
|---|---|---|
| `mysql_root_password` | mysql, backup/restore | random hex |
| `mysql_app_password` | mysql init | random hex (user `tc_app`, DML only) |
| `mysql_migrate_password` | mysql init | random hex (user `tc_migrate`, DDL) |
| `database_url` | api, engine | `mysql://tc_app:…@mysql:3306/trading_center` |
| `database_url_migrate` | migrate | `mysql://tc_migrate:…@mysql:3306/trading_center` |
| `engine_public_key` | api | X25519 public key (base64) — API can seal credentials, never open them |
| `engine_secret_key` | engine only | X25519 secret key (base64) — opens sealed broker credentials |
| `vapid_public_key` | api | Web Push VAPID public key (base64url) |
| `vapid_private_key` | engine only | Web Push VAPID private key |

Broker API keys are **not** stored here: they are entered once in the app's onboarding,
sealed with `engine_public_key`, and stored encrypted in MySQL.

Losing `engine_secret_key` makes stored broker credentials unrecoverable (re-enter them in
onboarding). Back it up offline together with the age backup identity.
