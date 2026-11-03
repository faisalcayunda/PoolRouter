//! Header constraints that the `params(..)`/`headers(..)` attributes cannot
//! express, applied as `Modify` passes after the documents are assembled.

/// `Idempotency-Key` bounds (ADR-0001 section 4.1): 1–128 printable ASCII
/// characters. The attribute syntax only carries a type and a description, so
/// `minLength`, `maxLength` and `pattern` are attached here.
pub const IDEMPOTENCY_POLA: &str = "^[ -~]{1,128}$";

/// Apply the `Idempotency-Key` bounds to every header of that name.
pub fn rapi_idempotency_key(openapi: &mut utoipa::openapi::OpenApi) {
    use utoipa::openapi::schema::{ObjectBuilder, Type};

    let skema = || {
        ObjectBuilder::new()
            .schema_type(Type::String)
            .min_length(Some(1))
            .max_length(Some(128))
            .pattern(Some(IDEMPOTENCY_POLA))
            .build()
    };

    for item in openapi.paths.paths.values_mut() {
        for operation in [
            &mut item.get,
            &mut item.put,
            &mut item.post,
            &mut item.delete,
            &mut item.options,
            &mut item.head,
            &mut item.patch,
            &mut item.trace,
        ]
        .into_iter()
        .flatten()
        {
            let Some(params) = operation.parameters.as_mut() else {
                continue;
            };
            for param in params.iter_mut() {
                let utoipa::openapi::RefOr::T(param) = param else {
                    continue;
                };
                if param.name.eq_ignore_ascii_case("Idempotency-Key") {
                    param.required = utoipa::openapi::Required::False;
                    param.schema = Some(skema().into());
                }
            }
        }
    }
}

/// Schemas that must be closed objects (`additionalProperties: false`).
///
/// `#[serde(deny_unknown_fields)]` is a serde concern and does not reach the
/// generated schema, and `additional_properties` in this utoipa version only
/// accepts a free-form map. The set is applied here instead.
pub const TERTUTUP: &[&str] = &[
    "SubmitRequest",
    "FieldInput",
    "FieldMetadata",
    "HasilElemen",
    "SyncResult",
    "EnqueueResponse",
    "RequestStatus",
    "EndpointKontrak",
    "TokenResponse",
    "SesiStatus",
    "Me",
    "DownstreamPool",
    "DownstreamPoolConfig",
    "ProviderConfig",
    "Client",
    "ClientConfig",
    "Credential",
    "WebhookLog",
];

/// Mark every schema in [`TERTUTUP`] as `additionalProperties: false`.
pub fn rapi_objek_tertutup(openapi: &mut utoipa::openapi::OpenApi) {
    use utoipa::openapi::schema::AdditionalProperties;

    let Some(components) = openapi.components.as_mut() else {
        return;
    };
    for nama in TERTUTUP {
        let Some(utoipa::openapi::RefOr::T(utoipa::openapi::Schema::Object(obj))) =
            components.schemas.get_mut(*nama)
        else {
            continue;
        };
        obj.additional_properties = Some(Box::new(AdditionalProperties::FreeForm(false)));
    }
}
