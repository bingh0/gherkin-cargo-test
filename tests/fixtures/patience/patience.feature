Feature: Patience
  Scenario: a light that never turns green
    Given a value of 1
    When the light turns green
    Then the value is 1
