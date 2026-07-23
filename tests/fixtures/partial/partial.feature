Feature: Partial
  One bound scenario, one pending plain scenario, and one pending outline —
  the shape scenario-scoped wip exists for: the ratchet stays tight on
  "ready" while the two pending constructs are explicitly held open.

  Scenario: ready
    Given a bound step

  Scenario: pending thing
    Given an unbuilt interface responds

  Scenario Outline: pending sweep <k>
    Given sweep case <k> runs on the unbuilt interface

    Examples:
      | k |
      | 1 |
      | 2 |
