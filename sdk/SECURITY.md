# Security policy

## Supported versions

Before the first npm publication, security fixes are made on the current
release-candidate branch. After publication, the latest minor release line is
the supported line unless release notes say otherwise.

## Report a vulnerability privately

Do not open a public issue for a suspected vulnerability.

Use the repository's
[private vulnerability reporting form](https://github.com/SPACE999978/ameba_sdk/security/advisories/new).
The repository owner must enable GitHub private vulnerability reporting before
the first public release. If that form is unavailable, open a public issue
containing no sensitive details and ask the maintainers to provide a private
reporting channel.

Include, when possible:

- the affected SDK version or commit;
- the affected entry point and method;
- reproduction steps or a minimal proof of concept;
- security impact and required preconditions; and
- any suggested mitigation.

Never include private keys, seed phrases, signed transactions, npm tokens,
production credentials, or personal data in a report.

## Security boundaries

The SDK never discovers wallets, signs or submits transactions. A report is
security relevant when it can make the SDK build an instruction or account
list that differs from the futures programs' interface, misdecode an account
or event in a way that misstates funds or positions, or expose credentials.

Ordinary usage questions and non-sensitive bugs belong in the channels listed
in [SUPPORT.md](./SUPPORT.md).
