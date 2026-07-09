# The crate's own acceptance demo: arithmetic on a typed World, Background,
# Scenario Outline expansion, step data tables, and a self-proving @skip.
Feature: Counter
  As a nonprogrammer running an agent-driven workflow
  I want scenarios to compile into cargo test trials
  So that acceptance criteria and unit tests share one command

  Background:
    Given a counter at 0

  Scenario: increment once
    When I add 5
    Then the counter is 5

  Scenario Outline: repeated increments
    When I add <a>
    And I add <b>
    Then the counter is <total>
    Examples:
      | a  | b  | total |
      | 1  | 2  | 3     |
      | 10 | 20 | 30    |

  Scenario: data tables reach the step
    When I add these amounts
      | amount |
      | 3      |
      | 4      |
    Then the counter is 7

  # The bound step panics if it ever runs: skip means "don't run" — and the
  # binding guard still requires it to be bound ("don't run" ≠ "don't bind").
  @skip
  Scenario: skipped scenarios do not run
    When I detonate
