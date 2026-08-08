# dottyenv

Validate `.env` files against a declared schema.

`.env.example` is untyped text. `dottyenv.toml` says which variables are required,
what they should look like, and **where to go get them**, so a missing variable
fails at startup with an actionable message instead of at runtime with a stack trace.

```
$ dottyenv check
✗ 2 problems in .env

  MISSING   STRIPE_SECRET_KEY
            Server-side Stripe key. Use sk_test_ locally.
            → https://dashboard.stripe.com/apikeys

  INVALID   DATABASE_URL
            expected ^postgres(ql)?://, got "mysql://localhost/app"

  8 of 10 required variables OK
```

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
description = "Primary Postgres connection string"

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
