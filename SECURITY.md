# Security Policy

## Supported Versions

Security fixes are handled on the current `main` branch and the latest tagged release.

## Reporting a Vulnerability

Please report suspected vulnerabilities privately by opening a GitHub security advisory for this repository if available, or by contacting the maintainer directly.

Do not open a public issue for secrets, private runtime data, or exploitable vulnerabilities.

Include:

- affected commit, tag, or branch;
- clear reproduction steps;
- expected impact;
- whether any secret, runtime memory dump, or personal data may be involved.

## Secret Handling

Runtime secrets and local memory are intentionally excluded from git:

- `.env`, `.env.*`
- `config/*.secrets.toml`
- `hosts/*/runtime/`
- generated `memory/` contents except placeholder `README.md` files

The repository uses GitHub Actions gitleaks scanning on pushes and pull requests. Local contributors should install `pre-commit` and `gitleaks`, then run:

```powershell
pre-commit install
pre-commit run --all-files
```

If a real secret is ever committed, rotate that secret immediately even if it is removed in a later commit.
