Feature: Red
  A failing scenario next to a passing one: the manifest must record the
  failure honestly and still be written — the run is complete even when red.

  Scenario: fails
    Given a failing step

  Scenario: passes
    Given a passing step
