// Placeholder trust settings so the firmware builds without a property.
// Generate the real file for a property with:
//   aice-hotels property.json firmware pod-header firmware_signing.pk8 https://desk.property.local:8791 > pod-firmware/include/property_trust.h
// A pod built with these placeholders refuses to enrol.

#pragma once

static const char PROPERTY_CA_PEM[] = "";
static const char PROPERTY_FIRMWARE_PUBKEY_HEX[] = "";
static const char FACILITATOR_URL[] = "";
