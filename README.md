# dottyenv

Validate `.env` files against a declared schema.

`.env.example` is untyped text. `dottyenv.toml` says which variables are required,
what they should look like, and **where to go get them**, so a missing variable
fails at startup with an actionable message instead of at runtime with a stack trace.

```
$ dottyenv check
✗ 3 problems in .env

  INVALID   DATABASE_URL
            expected ^postgres(ql)?://, got "<redacted, 47 chars>"
            PostgreSQL connection string

  INVALID   LOG_LEVEL
            expected one of [debug, info, warn, error], got "verbose"

  MISSING   STRIPE_SECRET_KEY
            Stripe secret key. Use sk_test_ locally.
            → https://dashboard.stripe.com/apikeys

  4 of 6 required variables OK
```

`DATABASE_URL` is marked `secret`, so its value is reported by length only. A
connection string carries a password, and an error message is the last place it
should surface. `LOG_LEVEL` is not a secret, so its value is shown.

## Install

Every path below installs a prebuilt binary. No Rust toolchain required.

**Homebrew** (macOS and Linux):

```bash
brew tap mfung/tap
brew install dottyenv
```

**Debian / Ubuntu:**

```bash
curl -LO https://github.com/mfung/dottyenv/releases/latest/download/dottyenv_0.1.1-1_amd64.deb
sudo dpkg -i dottyenv_0.1.1-1_amd64.deb
```

**RHEL / Fedora / Rocky:**

```bash
sudo rpm -i https://github.com/mfung/dottyenv/releases/latest/download/dottyenv-0.1.1-1.x86_64.rpm
```

Substitute `arm64` / `aarch64` in those filenames on ARM machines, and check
[Releases](https://github.com/mfung/dottyenv/releases) for the current version.

**Tarball:** download from Releases and put the binary on your `PATH`.

Linux artifacts are built against a glibc 2.28 floor, verified in CI by reading the
finished binary's dynamic symbols, so they run on RHEL 8, Ubuntu 20.04, and newer.
Every release ships a `SHA256SUMS`.

## Status

Early. `init`, `check`, and `list` work; `scan` is not implemented yet.

## Getting started

Point `init` at an existing `.env` (or `.env.example`) and it writes a schema,
filling in patterns and source URLs for providers it recognises:

```bash
dottyenv init      # writes dottyenv.toml
dottyenv check     # validate .env against it
```

Review the generated file before committing it. Anything `init` could not infer is
left as a `# TODO` comment rather than guessed at. A wrong pattern is worse than
no pattern.

## Schema

```toml
[vars.DATABASE_URL]
required    = true
pattern     = "^postgres(ql)?://"
description = "PostgreSQL connection string"
secret      = true

[vars.LOG_LEVEL]
required = false
default  = "info"
one_of   = ["debug", "info", "warn", "error"]
```

Commit `dottyenv.toml`. It holds no secrets, only their shape.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | All required variables present and valid |
| 1 | Validation failure |
| 2 | Usage error |
| 3 | Config error (schema missing or unparseable) |

## What it is not

Not an encryption tool (use SOPS or dotenvx), not a hosted secret store, and not a
replacement for direnv. It validates; that's all.

## License

MIT
