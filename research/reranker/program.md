# Pre-registered program: logistic reranker baseline

Status: **not started.** The corpus does not yet contain a ranking problem
(see `README_RU.md`). This file fixes what will be done, before the data
exists, so that a later run cannot quietly redefine success.

## Admission gate

The experiment starts only when a single evaluation run reports, under the
shipped `auto` routing policy:

* `reranker_readiness.examples_with_at_least_two_distinct_ast_candidates >= 50`
* `reranker_readiness.examples_where_gold_is_present_but_not_first >= 20`
* at least 10 such examples in each of `train` and `validation`

These thresholds are part of this program's version. Lowering one to make an
experiment possible requires a new version and a written reason.

## Model space

Deliberately small, local and inspectable. No PyTorch, no TensorFlow, no
pretrained download, no network access at any point.

* features: sparse character n-grams (3–5) of the transcript, plus structural
  features already recorded per candidate — source pass, requested domain,
  structural confidence, warning codes, candidate order, whether the AST is
  structurally valid;
* model: logistic regression, or a linear pairwise ranker over
  (correct, incorrect) candidate pairs from the same utterance;
* optimiser: deterministic SGD, fixed seed, fixed epoch count;
* artefact: a JSON model of a few hundred kilobytes at most.

## Files

```
research/reranker/
├── prepare.py    # corpus + evaluator output -> feature/label rows
├── train.py      # deterministic training, writes model.json
├── evaluate.py   # scores the deterministic order against the learned order
└── model.py      # feature extraction and the linear model, shared by all three
```

## Comparison

The only claim allowed is a paired comparison on the same examples:

```
deterministic order   vs   logistic reranker
```

reported with a paired 95% confidence interval, together with the safety
metrics from the same run — `FalseScientificRewriteRate`, the severity
histogram and `Coverage`. A gain in selection accuracy that costs safety is
not a win.

Five pre-recorded seeds; identical seeds must reproduce identical weights and
identical predictions, and that is asserted by a test rather than claimed.

## What this program does not authorise

* shipping the model into the application;
* touching the evaluation harness, the split manifest, the canonical AST
  contract or the severity taxonomy to improve a number;
* interpreting a falling training loss as a product improvement;
* a Transformer. That comes after logistic and MLP baselines have been beaten
  on equal terms, not before.
