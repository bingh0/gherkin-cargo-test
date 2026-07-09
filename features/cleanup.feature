# ctx.defer usage: scenario-scoped cleanup that runs even when a step fails
# (the run-on-failure and LIFO semantics are proven in tests/conformance.rs
# and tests/guards-proof.rs; this feature demonstrates the API).
Feature: Scenario cleanup
  Scenario: a scratch dir is created and cleaned up
    Given a scratch dir with a file named "probe.txt"
    Then the scratch file "probe.txt" exists
