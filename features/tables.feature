# DataTable surface: hashes / rows_hash, and Gherkin cell escapes.
Feature: Data tables
  Scenario: hashes give one map per row
    Given these users
      | name | role  |
      | ada  | admin |
      | bob  | dev   |
    Then user "ada" has role "admin"
    And there are 2 users

  Scenario: rows_hash gives a two-column key map
    Given this config
      | retries | 3    |
      | mode    | fast |
    Then config "retries" is "3"

  Scenario: cells honor pipe and backslash escapes
    Given these users
      | name    | role    |
      | pipe\|r | c:\temp |
    Then user "pipe|r" has role "c:\temp"
