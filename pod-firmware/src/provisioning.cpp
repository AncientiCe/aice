// Pod identity, enrolment with the property facilitator, and signed updates.

#include "provisioning.h"

#include <ArduinoJson.h>
#include <HTTPClient.h>
#include <Preferences.h>
#include <Update.h>
#include <WiFi.h>
#include <WiFiClientSecure.h>
#include <esp_system.h>
#include <mbedtls/md.h>
#include <mbedtls/pk.h>
#include <mbedtls/sha256.h>

#if __has_include("pod_config.h")
#include "pod_config.h"
#else
#include "pod_config.example.h"
#endif
#if __has_include("property_trust.h")
#include "property_trust.h"
#else
#include "property_trust.example.h"
#endif
#include "pod_defaults.h"

static const char *PREFS_NAMESPACE = "aice";
static const char *PREFS_NONCE = "nonce";
static const char *PREFS_TOKEN = "token";

// DER SubjectPublicKeyInfo prefix for an uncompressed P-256 point (65 bytes follow).
static const uint8_t P256_SPKI_PREFIX[] = {
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01,
    0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
};

static String random_hex(size_t bytes) {
    static const char *DIGITS = "0123456789abcdef";
    String out;
    out.reserve(bytes * 2);
    for (size_t i = 0; i < bytes; ++i) {
        uint8_t value = (uint8_t)(esp_random() & 0xFF);
        out += DIGITS[value >> 4];
        out += DIGITS[value & 0x0F];
    }
    return out;
}

static String stored_nonce() {
    Preferences prefs;
    prefs.begin(PREFS_NAMESPACE, false);
    String nonce = prefs.getString(PREFS_NONCE, "");
    if (nonce.length() < 32) {
        // First boot: this secret proves the pod's identity at enrolment.
        nonce = random_hex(32);
        prefs.putString(PREFS_NONCE, nonce);
    }
    prefs.end();
    return nonce;
}

static bool hex_to_bytes(const char *hex, uint8_t *out, size_t out_len) {
    size_t len = strlen(hex);
    if (len != out_len * 2) {
        return false;
    }
    for (size_t i = 0; i < out_len; ++i) {
        char pair[3] = {hex[2 * i], hex[2 * i + 1], 0};
        char *end = nullptr;
        long value = strtol(pair, &end, 16);
        if (end != pair + 2) {
            return false;
        }
        out[i] = (uint8_t)value;
    }
    return true;
}

String pod_device_id() {
    if (strlen(DEVICE_ID) > 0) {
        return String(DEVICE_ID);
    }
    String mac = WiFi.macAddress();
    mac.replace(":", "");
    mac.toLowerCase();
    return "pod-" + mac;
}

bool pod_trust_configured() {
    return strlen(PROPERTY_CA_PEM) > 0 && strlen(PROPERTY_FIRMWARE_PUBKEY_HEX) == 130 &&
           strlen(FACILITATOR_URL) > 0;
}

void pod_factory_reset_if_held(int pin) {
    uint32_t started = millis();
    while (digitalRead(pin) == LOW) {
        if (millis() - started >= FACTORY_RESET_HOLD_MS) {
            Preferences prefs;
            prefs.begin(PREFS_NAMESPACE, false);
            prefs.clear();
            prefs.end();
            Serial.println("[aice-pod] factory reset: identity forgotten");
            return;
        }
        delay(20);
    }
}

String pod_load_token() {
    Preferences prefs;
    prefs.begin(PREFS_NAMESPACE, true);
    String token = prefs.getString(PREFS_TOKEN, "");
    prefs.end();
    return token;
}

static void save_token(const String &token) {
    Preferences prefs;
    prefs.begin(PREFS_NAMESPACE, false);
    prefs.putString(PREFS_TOKEN, token);
    prefs.end();
}

void pod_clear_token() {
    Preferences prefs;
    prefs.begin(PREFS_NAMESPACE, false);
    prefs.remove(PREFS_TOKEN);
    prefs.end();
}

EnrollResult pod_enroll(bool lost_token) {
    if (!pod_trust_configured()) {
        Serial.println("[aice-pod] no property_trust.h: build this pod for a property first");
        return EnrollResult::Failed;
    }
    WiFiClientSecure client;
    client.setCACert(PROPERTY_CA_PEM);
    HTTPClient http;
    if (!http.begin(client, String(FACILITATOR_URL) + "/api/devices/enroll")) {
        return EnrollResult::Failed;
    }
    http.addHeader("Content-Type", "application/json");
    JsonDocument request;
    request["device_id"] = pod_device_id();
    request["nonce"] = stored_nonce();
    request["firmware"] = FIRMWARE_VERSION;
    request["lost_token"] = lost_token;
    String body;
    serializeJson(request, body);
    int status = http.POST(body);
    String response = http.getString();
    http.end();

    switch (status) {
        case 202:
            return EnrollResult::Pending;
        case 403:
            return EnrollResult::Revoked;
        case 409:
            return EnrollResult::Conflict;
        case 200: {
            JsonDocument doc;
            if (deserializeJson(doc, response)) {
                return EnrollResult::Failed;
            }
            const char *token = doc["token"];
            if (token == nullptr || strlen(token) < 32) {
                // Active, but the token was already handed out: ask for a new one.
                return lost_token ? EnrollResult::Failed : pod_enroll(true);
            }
            save_token(String(token));
            Serial.printf("[aice-pod] enrolled in room %s\n", (const char *)doc["room"]);
            return EnrollResult::Enrolled;
        }
        default:
            Serial.printf("[aice-pod] enrol failed: HTTP %d\n", status);
            return EnrollResult::Failed;
    }
}

static bool signature_valid(const uint8_t hash[32], const char *signature_hex) {
    size_t sig_hex_len = strlen(signature_hex);
    if (sig_hex_len == 0 || sig_hex_len % 2 != 0 || sig_hex_len > 2 * 80) {
        return false;
    }
    uint8_t signature[80];
    size_t sig_len = sig_hex_len / 2;
    if (!hex_to_bytes(signature_hex, signature, sig_len)) {
        return false;
    }
    uint8_t spki[sizeof(P256_SPKI_PREFIX) + 65];
    memcpy(spki, P256_SPKI_PREFIX, sizeof(P256_SPKI_PREFIX));
    if (!hex_to_bytes(PROPERTY_FIRMWARE_PUBKEY_HEX, spki + sizeof(P256_SPKI_PREFIX), 65)) {
        return false;
    }
    mbedtls_pk_context pk;
    mbedtls_pk_init(&pk);
    bool valid = mbedtls_pk_parse_public_key(&pk, spki, sizeof(spki)) == 0 &&
                 mbedtls_pk_verify(&pk, MBEDTLS_MD_SHA256, hash, 32, signature, sig_len) == 0;
    mbedtls_pk_free(&pk);
    return valid;
}

void pod_check_for_update(const String &token) {
    if (!pod_trust_configured() || token.length() == 0) {
        return;
    }
    WiFiClientSecure client;
    client.setCACert(PROPERTY_CA_PEM);
    HTTPClient http;
    String manifest_url = String(FACILITATOR_URL) + "/api/firmware/manifest?current=" + FIRMWARE_VERSION;
    if (!http.begin(client, manifest_url)) {
        return;
    }
    http.addHeader("Authorization", "Bearer " + token);
    int status = http.GET();
    if (status != 200) {
        http.end();
        return;
    }
    JsonDocument manifest;
    DeserializationError parse_error = deserializeJson(manifest, http.getString());
    http.end();
    if (parse_error) {
        return;
    }
    String version = manifest["version"] | "";
    String image_path = manifest["url"] | "";
    String signature_hex = manifest["signature_hex"] | "";
    size_t size = manifest["size"] | 0;
    if (version.length() == 0 || image_path.length() == 0 || size == 0) {
        return;
    }
    Serial.printf("[aice-pod] update %s offered (%u bytes)\n", version.c_str(), (unsigned)size);

    HTTPClient download;
    if (!download.begin(client, String(FACILITATOR_URL) + image_path)) {
        return;
    }
    download.addHeader("Authorization", "Bearer " + token);
    if (download.GET() != 200 || !Update.begin(size)) {
        download.end();
        return;
    }
    WiFiClient *stream = download.getStreamPtr();
    mbedtls_sha256_context sha;
    mbedtls_sha256_init(&sha);
    mbedtls_sha256_starts(&sha, 0);
    uint8_t buffer[1024];
    size_t received = 0;
    uint32_t last_data = millis();
    while (received < size && millis() - last_data < 15000) {
        size_t available = stream->available();
        if (available == 0) {
            delay(5);
            continue;
        }
        size_t want = min(min(available, sizeof(buffer)), size - received);
        int read = stream->readBytes(buffer, want);
        if (read <= 0) {
            continue;
        }
        mbedtls_sha256_update(&sha, buffer, (size_t)read);
        if (Update.write(buffer, (size_t)read) != (size_t)read) {
            break;
        }
        received += (size_t)read;
        last_data = millis();
    }
    uint8_t hash[32];
    mbedtls_sha256_finish(&sha, hash);
    mbedtls_sha256_free(&sha);
    download.end();

    if (received != size || !signature_valid(hash, signature_hex.c_str())) {
        Serial.println("[aice-pod] update rejected: incomplete or badly signed");
        Update.abort();
        return;
    }
    if (!Update.end(true)) {
        Serial.printf("[aice-pod] update failed: %s\n", Update.errorString());
        return;
    }
    Serial.printf("[aice-pod] update %s installed; restarting\n", version.c_str());
    delay(200);
    ESP.restart();
}
