# CHANGELOG (MailLaser)


<a name="v1.1.1"></a>
## [v1.1.1](https://github.com/Govcraft/mail-laser/compare/v1.1.0...v1.1.1)

> 2025-04-08

### Bug Fixes

* **smtp:** correctly handle non-TLS sessions and sender address parsing


<a name="v1.1.0"></a>
## [v1.1.0](https://github.com/Govcraft/mail-laser/compare/v1.0.0...v1.1.0)

> 2025-04-08

### Features

* **smtp:** extract sender name from Reply-To header


<a name="v1.0.0"></a>
## [v1.0.0](https://github.com/Govcraft/mail-laser/compare/v0.5.2...v1.0.0)

> 2025-04-06


<a name="v0.5.2"></a>
## [v0.5.2](https://github.com/Govcraft/mail-laser/compare/v0.5.1...v0.5.2)

> 2025-04-06

### Features

* **parser:** handle multipart emails and generate text body using mailparse


<a name="v0.5.1"></a>
## [v0.5.1](https://github.com/Govcraft/mail-laser/compare/v0.5.0...v0.5.1)

> 2025-04-06

### Features

* **parser:** handle multipart emails and generate text body using mailparse


<a name="v0.5.0"></a>
## [v0.5.0](https://github.com/Govcraft/mail-laser/compare/v0.4.0...v0.5.0)

> 2025-04-06

### Features

* **parser:** use Content-Type for HTML detection and include HTML body
* **parser:** include HTML body in webhook payload and strip for text body


<a name="v0.4.0"></a>
## [v0.4.0](https://github.com/Govcraft/mail-laser/compare/v0.3.0...v0.4.0)

> 2025-04-06

### Bug Fixes

* **build:** correctly include config tests from separate file


<a name="v0.3.0"></a>
## [v0.3.0](https://github.com/Govcraft/mail-laser/compare/v0.2.12...v0.3.0)

> 2025-04-06

### Features

* **smtp:** add STARTTLS support using rustls


<a name="v0.2.12"></a>
## [v0.2.12](https://github.com/Govcraft/mail-laser/compare/v0.2.11...v0.2.12)

> 2025-04-05

### Code Refactoring

* **ci:** simplify release notes handling


<a name="v0.2.11"></a>
## [v0.2.11](https://github.com/Govcraft/mail-laser/compare/v0.2.10...v0.2.11)

> 2025-04-05

### Bug Fixes

* **ci:** handle empty release notes and pass body via output


<a name="v0.2.10"></a>
## [v0.2.10](https://github.com/Govcraft/mail-laser/compare/v0.2.9...v0.2.10)

> 2025-04-05

### Bug Fixes

* **ci:** handle empty release notes and pass body via output


<a name="v0.2.9"></a>
## [v0.2.9](https://github.com/Govcraft/mail-laser/compare/v0.2.8...v0.2.9)

> 2025-04-05

### Bug Fixes

* **ci:** ensure release-notes.md exists before generation


<a name="v0.2.8"></a>
## [v0.2.8](https://github.com/Govcraft/mail-laser/compare/v0.2.7...v0.2.8)

> 2025-04-05

### Bug Fixes

* **ci:** separate full changelog from release notes


<a name="v0.2.7"></a>
## [v0.2.7](https://github.com/Govcraft/mail-laser/compare/v0.2.6...v0.2.7)

> 2025-04-05


<a name="v0.2.6"></a>
## [v0.2.6](https://github.com/Govcraft/mail-laser/compare/v0.2.5...v0.2.6)

> 2025-04-05

### Bug Fixes

* **ci:** use correct git-chglog argument for existing tags


<a name="v0.2.5"></a>
## [v0.2.5](https://github.com/Govcraft/mail-laser/compare/v0.2.4...v0.2.5)

> 2025-04-05

### Bug Fixes

* **test:** prevent parallel execution interference in config tests

### Code Refactoring

* **health:** Migrate health check server from Axum to Hyper

### Features

* **config:** add info logging for loaded configuration values


<a name="v0.2.4"></a>
## [v0.2.4](https://github.com/Govcraft/mail-laser/compare/v0.2.3...v0.2.4)

> 2025-04-05


<a name="v0.2.3"></a>
## [v0.2.3](https://github.com/Govcraft/mail-laser/compare/v0.2.2...v0.2.3)

> 2025-04-05

### Bug Fixes

* **docker:** ensure application runs correctly in Docker container


<a name="v0.2.2"></a>
## [v0.2.2](https://github.com/Govcraft/mail-laser/compare/v0.2.1...v0.2.2)

> 2025-04-05

### Features

* **error-handling:** improve panic logging and enable unwinding


<a name="v0.2.1"></a>
## [v0.2.1](https://github.com/Govcraft/mail-laser/compare/v0.2.0...v0.2.1)

> 2025-04-05

### Code Refactoring

* improve config loading robustness and cleanup Dockerfile


<a name="v0.2.0"></a>
## [v0.2.0](https://github.com/Govcraft/mail-laser/compare/v0.1.0...v0.2.0)

> 2025-04-05

### Features

* **health:** add basic HTTP health check endpoint


<a name="v0.1.0"></a>
## v0.1.0

> 2025-04-04

### Bug Fixes

* **build:** correct Dockerfile syntax and certificate placement
* **config:** change default SMTP port to 2525 to avoid permission errors
* **smtp:** correct DATA phase handling and loop termination
* **webhook:** update hyper client to 1.x API

### Code Refactoring

* rename project to mail_laser

### Features

* **build:** add static musl Docker build using rustls
* **ci:** add workflow to build and publish Docker image to GHCR on tag
* **webhook:** use dynamic user-agent from Cargo manifest

