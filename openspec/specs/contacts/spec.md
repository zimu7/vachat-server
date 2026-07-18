# Contacts

## Purpose

Defines the visibility, lifecycle, and direct-message gating of contacts
between users in the system. Every registered user is visible in a
requester's contact list by default; blocking and deleting are independent
operations that affect visibility and messaging respectively.

## Requirements

### Requirement: Contacts visible by default

The system SHALL make every registered user visible in the requesting user's
contact list by default, without requiring an explicit "add" action. A user
SHALL appear in `GET /contacts` as long as they are not deleted by the
requester and are not the requester themselves.

For a target user that has no `contacts` record with the requester, the
contact's `status` field SHALL be `"default"`. For a target user with an
existing `contacts` record of `status = 1`, the `status` field SHALL be
`"added"`.

#### Scenario: User with no prior contact record is visible

- **WHEN** user A calls `GET /contacts` and user B exists but has no
  `contacts` row between A and B
- **THEN** user B appears in the returned list with `status = "default"`

#### Scenario: Requester is excluded from own contact list

- **WHEN** user A calls `GET /contacts`
- **THEN** user A does not appear in the returned list

#### Scenario: Explicitly added contact retains added status

- **WHEN** user A has previously added user B (`status = 1`) and calls
  `GET /contacts`
- **THEN** user B appears in the returned list with `status = "added"`

### Requirement: Blocked contacts are visible but cannot receive direct messages

Blocking (`contacts.status = 2`) SHALL NOT hide a user from the requester's
`GET /contacts` response. A blocked user SHALL appear in the list with
`status = "blocked"`. Blocking is the only mechanism that suppresses direct
messaging; it does not affect list visibility.

#### Scenario: Blocked user remains visible

- **WHEN** user A has blocked user B (`status = 2`) and calls `GET /contacts`
- **THEN** user B appears in the returned list with `status = "blocked"`

#### Scenario: Unblocked user returns to default

- **WHEN** user A unblocks user B (previously `status = 2`) and then calls
  `GET /contacts`
- **THEN** user B appears in the returned list with `status = "default"`

### Requirement: Deleted contacts are hidden

The `remove` action of `POST /update_contact_status` SHALL delete the contact
by persisting a `contacts` record with `status = 3` (deleted). The system
SHALL NOT include a deleted user (`contacts.status = 3`) in the requesting
user's `GET /contacts` response. Deletion is the only mechanism that makes a
user invisible in the contact list.

#### Scenario: Deleted user is excluded from contact list

- **WHEN** user A has deleted user B (`status = 3`) and calls `GET /contacts`
- **THEN** user B does not appear in the returned list

#### Scenario: Re-adding restores a deleted contact

- **WHEN** user A re-adds user B (previously `status = 3`) and then calls
  `GET /contacts`
- **THEN** user B appears in the returned list with `status = "added"`

### Requirement: Deletion does not block direct messages

Deleting a contact (`contacts.status = 3`) SHALL only affect list visibility
and SHALL NOT suppress direct messaging. Only a block (`contacts.status = 2`)
SHALL cause a direct message to be rejected.

#### Scenario: Sender deleted by recipient can still DM

- **WHEN** user A has deleted user B (`status = 3`) and user B sends a direct
  message to user A
- **THEN** the system accepts the send (it is not rejected as a block would
  be)

### Requirement: Unblock returns to default visible state

The `unblock` action of `POST /update_contact_status` SHALL remove the
block and leave the target user in the default-visible state, i.e. no
`contacts` record remains with `status = 1` solely as a side effect of
unblocking. An explicitly added contact (`status = 1`) is not altered by an
unblock action on a different user.

#### Scenario: Unblock clears the contacts record

- **WHEN** user A unblocks user B where the only `contacts` row between
  them had `status = 2`
- **THEN** no `contacts` row between A and B remains, and B is visible in
  `GET /contacts` with `status = "default"`

#### Scenario: Unblock on a non-blocked user is a no-op

- **WHEN** user A calls `unblock` on user B who is not blocked (no
  `contacts` row, `status = 1`, or `status = 3`)
- **THEN** the request succeeds and B's visibility/state is unchanged

### Requirement: Direct message blocked by recipient

The system SHALL reject any direct message sent to a recipient who has
blocked the sender (`contacts.status = 2`), returning HTTP 403 FORBIDDEN.
This behavior SHALL remain unchanged by the default-visible refactor.

#### Scenario: Sender blocked by recipient cannot DM

- **WHEN** user A sends a direct message to user B and B has blocked A
  (`status = 2`)
- **THEN** the system rejects the send with HTTP 403 FORBIDDEN
