Feature: Unbound
  Scenario: one step is not bound
    Given a bound step
    When an unbound step with 42 and "text"
