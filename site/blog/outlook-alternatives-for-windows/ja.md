---
title: Windows の Outlook 代替 — 2026 年の静かな短いリスト
description: Windows で新しい Outlook の代わりに何を使うか — 無料、有料、オープンソース。各ツールの得意、省略するもの、そして Outlook に残るのが依然正しい場面。
date: 2026-10-27
updated: 2026-10-28
image: https://skim-tech.com/social-preview.jpg
related: [best-free-email-client-for-windows-11, open-source-email-client-for-windows, thunderbird-alternatives-in-2026]
---

Windows の「新しい」Outlook はネイティブの外殻の中の Web アプリで、Gmail・iCloud・Yahoo の認証情報を Microsoft のサーバー経由にして一箇所にまとめます。それで良い人もいます。残りはこういう記事を読みます。

## 「Outlook の代替」の意味

Outlook は三つの仕事をします：Microsoft 365（Exchange）を読む、他のすべて（IMAP/SMTP）を読む、カレンダーとアドレス帳を同じ窓口に束ねる。Exchange 機能（共有メールボックス、委任）が必要なら、完全な代替は存在しません。ここのすべては IMAP 経由で Exchange と話します, メールは可、共有カレンダーは不可。

## 短いリスト

### Thunderbird（無料、オープンソース）

明らかな代替。アイドル 300–500 MB。

### eM Client（Free + Pro）

視覚的に Outlook に最も近い。Free は最大 2 アカウント。

### Mailspring（Free + Pro）

オープンソースのクライアント、Pro 機能は Mailspring のサーバーで IMAP 認証情報を保持。

### Spark（Free + Spark+）

「スマート受信箱」、有料で AI、メールは Readdle サーバー経由。

### Skim（無料、オープンソース）

[Skim](/) はネイティブ Rust コアの Windows 向け無料・オープンソースメールクライアント。~5 MB のインストーラ、管理者権限不要、IMAP で FTS5 検索付きのローカル SQLite へ同期。カレンダー、連絡先、ルールなし。[AI コパイロット](/#ai) はあなたのキーを使う. Anthropic、OpenRouter、OpenAI 互換エンドポイント、または Ollama/LM Studio のローカルモデル。サブスクなし。

「Windows の Outlook 代替」が、速く・ローカルで・キーボード優先で・毎月の請求のない AI を意味するなら Skim。カレンダー同梱が必須なら別のを。

### Betterbird（オープンソース）

Thunderbird のコミュニティフォーク、修正が速い。

### Mailbird（有料）

Windows のみ、Electron。無料プランなし、14 日トライアルのみ。

## 比較

| クライアント | 無料 | オープンソース | ネイティブ | ローカルのみ | カレンダー | AI |
|---|---|---|---|---|---|---|
| Thunderbird | はい | はい | はい | はい | はい | アドオン |
| eM Client | 2 アカウントまで | いいえ | はい | はい | はい | アドオン |
| Mailspring | クラウド付き | コア | いいえ | いいえ | いいえ | いいえ |
| Spark | はい | いいえ | いいえ | いいえ | はい | 有料 |
| Skim | はい | はい | はい | はい | いいえ | BYOK |
| Betterbird | はい | はい | はい | はい | はい | いいえ |

## Outlook が正解のまま残る場面

共有メールボックス、委任、スケジュールアシスタント、Intune の方針. Exchange が必要で、完全対応は Outlook のみ。

## FAQ

**Windows で最良の無料 Outlook 代替？** オールインワンなら Thunderbird、速くキーボード優先と任意の AI なら Skim。

**代替は Exchange に対応？** IMAP 経由のみ, メールは可、共有カレンダーは不可。

**Gmail は動く？** どれでも、Google ログインかアプリパスワード。

**モバイルは？** 本記事の対象外。Android/iOS では Spark、Edison、純正アプリ。

**本当に最も速いのは？** 日常使いで Skim と Thunderbird が機敏；Electron のクライアントは明らかに遅い。

[Skim をダウンロード →](https://github.com/nikserg/skim/releases/latest)
