Feature: Only
  @only
  Scenario: tries to use only
    Given a value of 1
    Then the value is 1

  Scenario: the untagged sibling
    Given a value of 2
    Then the value is 2
