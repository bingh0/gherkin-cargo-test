Feature: Defer
  Scenario: cleanup runs even though the last step fails
    Given cleanup is registered
    Then this step fails on purpose
