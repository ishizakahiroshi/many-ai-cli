# Synthetic approval oracle

Fixed Go revision: `21d0bc7935a2c4696fb89ccff2e324157a528c2d`.
The generator copies pure functions from approval summary, identity, native
screen detector, trigger phrases, provider masking and sessionlog ANSI stripping
into an explicit temporary package. Package qualifiers are removed; the native
adapter's generic summarizer is replaced with its exact copied implementation.
No live Hub, provider, transcript or private settings are involved.

Corpus: 32 risk/shell-syntax cases, four summaries, and 14 provider-native cases
(including intentionally absent detection, two-stage OpenCode, notices and
synthetic masking). Rust tests compare native signature, kind, question, context,
options and summary against Go values, in addition to separate record/epoch
state tests. Native process/UI and complete transcript polling acceptance are
separate and are not claimed by this corpus.
