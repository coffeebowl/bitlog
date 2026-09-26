# Deployment

Releases are deployed in the evening, after the shop traffic drops. #release

1. Tag the release.
2. Run the pipeline, it deploys to staging first.
3. Check the dashboards, then promote to production.

The webshop needs a cache purge afterwards, see [[webshop/checkout-flow]].
