# Chapter 18: Security Guardrails

Give an agent a file-writing tool and you have given it a file-writing tool. Everything in
this chapter is about narrowing what that sentence means.

Nami Mini's guardrails are small, explicit, and enforced in Rust — not prompted into
goodness.

## 1. Path Confinement

`WorkspaceGuard` is the whole story:

```rust
// src/workspace.rs
impl WorkspaceGuard {
    pub fn new(allowed_paths: Vec<PathBuf>, read_only: bool) -> Result<Self> {
        let mut allowed_roots = Vec::new();
        for path in allowed_paths {
            let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            allowed_roots.push(canonical);
        }
        Ok(Self { allowed_roots, read_only })
    }

    pub fn validate_path<P: AsRef<Path>>(&self, target: P, is_write: bool) -> Result<PathBuf> {
        if is_write && self.read_only {
            return Err(anyhow!(
                "File editing is disabled because the workspace is in Read-Only mode"
            ));
        }
        ...
        let is_allowed = self.allowed_roots.iter().any(|root| canonical_target.starts_with(root));
        if !is_allowed {
            return Err(anyhow!(
                "File access denied: path {:?} is outside the allowed scope", target_path
            ));
        }
        Ok(canonical_target)
    }
}
```

Three properties make this trustworthy rather than merely present:

- **Canonicalisation happens on both sides.** Roots are canonicalised at construction;
  targets are canonicalised before comparison — and when the target does not exist yet, its
  *parent* is canonicalised and the filename reattached. That last branch is what stops
  `./docs/../../secrets.env` from passing a naive string check.
- **The check is prefix-based on canonical paths**, so symlinks and `..` segments resolve
  before the comparison rather than after it.
- **`is_write` is an argument, not a guess.** Read tools pass `false`, write tools pass
  `true`, and read-only mode blocks only the latter. Reading remains available in a
  read-only workspace, which is the entire point of having one.

`starts_with` on path components does mean `./docs-private` is **not** treated as inside
`./docs` — component-wise, not character-wise. That is the correct behaviour and worth
stating, because the string-prefix version of this check is a famous vulnerability.

## 2. Two Enforcement Points, Two Scopes

This is the part to get right, and Chapter 9 flagged it: my filesystem tools and the
`adk-devtools` suite enforce different roots.

| Mechanism | Scope enforced | Read-only honoured |
|-----------|----------------|--------------------|
| `WorkspaceGuard` (→ `read_file`, `write_file`, `list_files`) | **All** of `allowed_paths` | Yes — blocks writes only |
| `Workspace::new` / `Workspace::read_only` (→ DevTools, `bash`) | **First** `allowed_paths` entry only | Partially — see below |

With the shipped config (`["./", "../shared_docs"]`), that means: `../shared_docs` is
reachable through my three filesystem tools and invisible to DevTools. Plan around which
toolset a specialist actually has.

**Important:** `read_only` restricts the DevTools *file* operations. Bash is enabled
unconditionally:

```rust
// src/agent.rs
ws = ws.allow_bash(true);
```

A read-only workspace with `devtools` attached still gives the model a shell inside the
workspace root. `read_only` is a guardrail against accidental writes, not a sandbox. If you
want genuinely read-only, attach `filesystem` instead of `devtools`, or give the specialist
no tools at all (Chapter 16).

## 3. The Server

The HTTP surface is `adk-server` with loopback-first defaults: the default `server.host` is
`127.0.0.1`, and unlisted hosts or cross-origin requests get 403 from the host guard. That
default is chosen deliberately — exposing Nami to a network should require an explicit
config change, not be the outcome of forgetting to set a host.

`NamiServer::start` binds before spawning and polls `/api/health` for readiness, so nothing
serves traffic in a half-initialised state.

## 4. Secrets

Keys come from the environment or `.env`, loaded once by `dotenvy` at the top of `main`.
Two rules the code follows:

- **Never print a key.** The startup warnings name the variable and nothing else:

  ```
  ⚠️  Warning: GOOGLE_API_KEY not found in .env or the environment
  ```

- **Never fall back to a real-looking value.** A missing provider key becomes the literal
  string `"mock_key"`, which is obviously wrong and immediately detected by the API — so
  misconfiguration surfaces as an authentication failure you can diagnose, not as silent
  degradation.

`.env` is gitignored. Treat `nami.json` as safe to commit and `.env` as never safe.

## 5. What Is Not Guarded

Stated plainly, because a security chapter that only lists controls is marketing:

- **No tool-approval step.** Every attached tool runs without asking you first. The SDK
  supports confirmation policies; Nami Mini does not enable them.
- **No output filtering.** No PII redaction, no content guardrails on responses.
- **No authentication on the API.** Loopback binding is the only thing in front of it. Do
  not bind to `0.0.0.0` on a network you do not control.
- **No sandbox around bash.** The command runs with your user's full OS permissions,
  constrained only by the workspace convention.
- **No per-tool rate or spend limits.**

## 6. Hardening, in Priority Order

If you are running me somewhere that matters, in the order the returns come:

1. `"read_only": true` in `workspace`, and prefer `filesystem` over `devtools` for any
   specialist that does not need a shell.
2. Remove entries from `allowed_paths`. Fewer roots is strictly better than trusted roots.
3. Bind the server to loopback only, and put real authentication in front if it must be
   reached remotely.
4. Disable specialists you are not using (Chapter 16) — every agent is attack surface.
5. Keep `.env` out of version control and rotate keys on schedule.

Next: the part of this that is about judgement rather than configuration.
