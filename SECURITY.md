# Security Policy

## Supported Versions

Security fixes are provided for the `main` branch and the latest stable releases of crates maintained in this repository.

Older versions generally do not receive security backports. Users of older releases should upgrade to the latest version to receive fixes.

## Reporting a Vulnerability

Please report vulnerabilities privately through GitHub Private Vulnerability Reporting:

https://github.com/hotpath-rs/hotpath/security/advisories/new

Do not disclose unpatched vulnerabilities in public issues, pull requests, or discussions.

A helpful report includes:

- Affected versions and enabled Cargo features
- Potential impact
- Steps to reproduce, ideally with a minimal example
- Relevant environment details (OS, Rust toolchain, async runtime)

## Disclosure and Remediation

Reports are investigated by the maintainer, and disclosure is coordinated with the reporter. Fixes are generally released as new versions on crates.io. When appropriate, a GitHub Security Advisory and a [RustSec](https://rustsec.org/) advisory may be published.

Responsible security research is appreciated, and reporters will be credited in the advisory unless they prefer to remain anonymous.
