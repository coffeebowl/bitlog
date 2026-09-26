---
aliases: ["PSP"]
---

# Payment provider

We are switching providers in October. The new API uses idempotency keys,
see [[webshop/checkout-flow]] for where they are generated. #payments

```sh
curl -X POST https://api.psp.example/v2/payments \
  -H "Idempotency-Key: $KEY"
```
