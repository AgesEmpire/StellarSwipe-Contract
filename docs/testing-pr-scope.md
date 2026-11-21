# Testing PR Scope

This branch adds deterministic event-schema integrity checks and generated reward/fee conservation regressions.

The schema check is network-free and fails on duplicate events, version mismatches,
unnamed or reordered body fields, unsupported types, and missing core contract families.
Reward cases include zero, boundary, and generated amounts; integer division residuals
remain with the funded pool until a successful claim, and unauthorized claims are atomic.
