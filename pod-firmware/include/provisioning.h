// Pod identity, enrolment with the property facilitator, and signed updates.

#pragma once

#include <Arduino.h>

enum class EnrollResult {
    Enrolled,  // token stored
    Pending,   // waiting for a supervisor to assign a room
    Revoked,   // a supervisor revoked this pod
    Conflict,  // enrolled before with another nonce (factory reset + revoke needed)
    Failed,    // network or server error; try again
};

// "pod-<wifi mac>" unless DEVICE_ID is set.
String pod_device_id();

// True when property_trust.h carries a CA, firmware key, and facilitator URL.
bool pod_trust_configured();

// Forget the nonce and token when `pin` is held low for FACTORY_RESET_HOLD_MS.
void pod_factory_reset_if_held(int pin);

String pod_load_token();
void pod_clear_token();

// One enrolment round. `lost_token` asks the facilitator for a fresh token
// (proven by the stored nonce) when this pod no longer has one.
EnrollResult pod_enroll(bool lost_token);

// Ask for an update; when one is offered, download it, check its ECDSA
// signature against PROPERTY_FIRMWARE_PUBKEY_HEX, flash it and restart.
// Returns normally when there is no update or it was rejected.
void pod_check_for_update(const String &token);
