Feature: Todo
  @todo
  Scenario: fails but is tolerated
    Given a step that panics
  Scenario: passes normally
    Given a value of 3
    Then the value is 3
