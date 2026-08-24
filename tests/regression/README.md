# Regression Test Suite

This directory contains the dedicated regression harness for known edge cases.

Each test corresponds to a previously discovered bug or failure scenario.

## How to run

From the repository root:

```bash
cd scripts
npm install
npm run test:regression
```

## Goals

- Document each bug with a regression test comment.
- Guard against reintroduction of known edge cases.
- Execute locally without deploying to testnet.
