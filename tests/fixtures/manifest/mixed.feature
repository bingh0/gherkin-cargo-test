Feature: Mixed
  One scenario per manifest status a green run can produce: passed, skipped,
  todo, unbound (held open by scenario-scoped wip), and two expanded outline
  rows — the shape the run manifest records.

  Scenario: passes
    Given a bound step

  @skip
  Scenario: skipped one
    Given a bound step

  @todo
  Scenario: todo one
    Given a bound step

  Scenario: pending thing
    Given an unbound step

  Scenario Outline: sweep <k>
    Given a bound step

    Examples:
      | k |
      | 1 |
      | 2 |
