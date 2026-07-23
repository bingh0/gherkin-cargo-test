Feature: Twins
  Two scenarios sharing one title: the rejection must fail the run while both
  copies still register and pass — rejection is additive, never narrowing.

  Scenario: the same name
    Given a value of 3
    Then the value is 3

  Scenario: the same name
    Given a value of 3
    Then the value is 3
