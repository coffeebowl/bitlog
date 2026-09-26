# Checkout flow

The checkout has three steps: cart, address, payment. Validation runs on
every step, the final price is only computed on the server. #checkout

## Payment

Payments go through the provider described in
[[webshop/payment-provider]]. Releases are rolled out as described in
[[infra/deployment]].

## Open questions

- [ ] Do we keep the guest checkout?
- [x] Round prices per line item, not per cart
