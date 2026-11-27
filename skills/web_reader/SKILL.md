---
name: web-reader
description: Reads a web page via URL and returns the raw HTML content. Use this when the user asks to fetch, read, or summarize a web page.
compatibility: Requires Aegis Runtime runtime (Wasm sandbox with Ed25519-signed intent envelopes)
metadata:
  author: aegis-runtime
  version: "0.1"
  runtime: wasm-sandboxed
  aegis-runtime-domains: "*"
---

# Web Reader

## Overview
Fetches the full HTML content of any publicly accessible web page given its URL. The returned HTML can then be processed by the orchestrator for summarization, data extraction, or content analysis. This skill has unrestricted domain access, making it the general-purpose tool for retrieving web content.

## When to use
- The user asks to read, fetch, or open a specific URL
- The user wants a summary of a web page
- Following up on links returned by the web-search skill
- Extracting specific information from a known web page

## Parameters
| Name | Type   | Required | Description                          |
|------|--------|----------|--------------------------------------|
| url  | string | yes      | The full URL of the web page to fetch |

## Examples

**Fetch a page:**
```json
{ "url": "https://example.com/article" }
```

**Summarize a news article:**
```json
{ "url": "https://www.reuters.com/technology/some-article" }
```

## Security
This skill runs in a WebAssembly sandbox with capability-based permissions.
Allowed domains: `*` (unrestricted)
Filesystem access: none
