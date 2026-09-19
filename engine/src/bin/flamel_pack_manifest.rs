use serde_json::json;

fn main() {
    let manifests = flamel::developer_packs::release_payloads()
        .expect("compiled first-party pack manifests must serialize");
    let document = json!({
        "schemaVersion": 1,
        "product": "Flamel Assistant first-party capability packs",
        "signatureRequired": true,
        "signatureContract": {
            "algorithm": "ed25519",
            "keyId": "flamel-release-v1",
            "canonicalPayload": "compact JSON serialization of each manifest object",
            "envelopeFields": ["manifest", "signature.algorithm", "signature.keyId", "signature.value"]
        },
        "manifests": manifests
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&document)
            .expect("compiled first-party pack release document must serialize")
    );
}
