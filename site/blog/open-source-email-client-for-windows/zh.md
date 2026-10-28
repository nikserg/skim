---
title: Windows 上的开源邮件客户端 — 2026 年的现实简短名单
description: 五款在 2026 年真正维护的 Windows 开源邮件客户端，附许可证、凭据存放位置，以及"开源"何时带有星号。
date: 2026-10-24
updated: 2026-10-28
image: https://skim-tech.com/social-preview.jpg
related: [best-free-email-client-for-windows-11, thunderbird-alternatives-in-2026, best-email-client-for-windows-2026]
---

邮件客户端着陆页上的"开源"含义多样。有时是"客户端 MIT 许可，无服务器，去读代码"。有时是"客户端开源，但有用的功能（搜索、延迟发送、读收据）跑在我们服务器上并保存你的凭据"。两者都合法，但不是一回事。

## 需要检查什么

三个问题：你的凭据存在哪里？服务器组件也是开源的吗？是否仍在维护（最后提交日期）？

## 简短名单

### Thunderbird（MPL 2.0）

事实上的标准。全部本地，没有供应商服务器在邮件路径中。Supernova + 2026 年统一收件箱。空闲 300–500 MB。

### Betterbird（MPL 2.0）

社区分支，修复更快。完全开源，无服务器。

### Skim（MIT）

[Skim](/) 是面向 Windows 的原生 Rust 邮件客户端，UI 为 Svelte 5 + WebView2。安装包 ~5 MB，不需管理员权限。IMAP 同步到带 FTS5 搜索的本地 SQLite；所有变更走本地 offline-first 队列--归档/删除/发送在 UI 里立刻完成，稍后与服务器对账。

[AI 功能](/#ai) 为 BYOK 且本地友好：Anthropic / OpenRouter / OpenAI 兼容端点，或 Ollama/LM Studio 的 `localhost:11434`。凭据与 API 密钥存于 Windows 凭据管理器。

范围：仅邮件。无日历、联系人、过滤器、PGP。

仓库：github.com/nikserg/skim。许可：MIT。

### Mailspring（客户端 GPL-3.0）

客户端开源，Pro 后端闭源且不可自托管。

### Claws Mail（GPL-3.0）

Linux 经典，也可在 Windows 运行。GTK，2008 风格的美学。

## 对比

| 客户端 | 许可 | 原生 UI | 云端 | 可自托管 | 仅邮件 |
|---|---|---|---|---|---|
| Thunderbird | MPL 2.0 | 是（Gecko） | 否 | N/A | 否 |
| Betterbird | MPL 2.0 | 是（Gecko） | 否 | N/A | 否 |
| Skim | MIT | 是（Rust+WebView2） | 否 | N/A | 是 |
| Mailspring | GPL-3.0（客户端） | Electron | Pro 需 | 否 | 是 |
| Claws Mail | GPL-3.0 | 是（GTK） | 否 | N/A | 大体是 |

## 开源在这里为何重要

邮件是你全部数字生活的密码重置向量。闭源客户端可能--有意或无意--外泄凭据或在服务器端建立索引。原生、开源、本地优先的客户端（Thunderbird、Betterbird、Skim）把信任面缩小到可读的应用与可调用的 API。

## FAQ

**Windows 2026 年最佳开源邮件客户端？** 什么都做选 Thunderbird，追求快选 Skim，想要 Thunderbird-不粗糙选 Betterbird。

**Thunderbird 便携版？** Portableapps.com。

**新版 Outlook 开源吗？** 否。

**Skim 有像 Thunderbird 的扩展生态吗？** 没有，有意为之。

**自托管 Mailspring 后端？** 不能，只有客户端开源。

**Thunderbird 有 AI 吗？** 未内置。社区插件可以。Skim 以你的密钥在产品内提供 AI--见 [AI](/#ai)。
