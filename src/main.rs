use namada_sdk::{
    core::types::address::Address,
    proof_of_stake::types::ValidatorState,
    Namada, NamadaImpl, wallet::fs::FsWalletUtils, masp::fs::FsShieldedUtils, io::NullIo,
    rpc,
};
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;
use axum::{
        routing::get,
        Router,
        extract::{Path, State},
        response::IntoResponse,
        Json,
        http::{header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE}, HeaderValue, Method},
    };
use tendermint_rpc::{HttpClient, Url};
use std::{
    sync::Arc,
    str::FromStr,
};
use dotenv::dotenv;

pub struct AppState {
    // http_client: HttpClient,
    namada_impl: NamadaImpl<HttpClient, FsWalletUtils, FsShieldedUtils, NullIo>,
}

impl AppState {
    pub async fn new(rpc_url: Url) -> Self {
        // setup namada_impl
        let http_client: HttpClient = HttpClient::new(rpc_url).unwrap();
        let wallet = FsWalletUtils::new("wallet".into());
        let shielded_ctx = FsShieldedUtils::new("masp".into());
        let null_io = NullIo;
        Self {
            namada_impl: NamadaImpl::new(http_client, wallet, shielded_ctx, null_io).await.unwrap(),
        }
    }

    pub fn get_client(&self) -> &HttpClient {
        &self.namada_impl.client()
    }
}

#[tokio::main]
async fn main() {
    dotenv().ok();

    let listener: TcpListener = TcpListener::bind("0.0.0.0:1317").await.expect("Could not bind to listen address.");
    let cors = CorsLayer::new()
        .allow_origin("http://localhost:1317".parse::<HeaderValue>().unwrap())
        .allow_methods([Method::GET])
        .allow_credentials(true)
        .allow_headers([AUTHORIZATION, ACCEPT, CONTENT_TYPE]);

    // eg: RPC="http://localhost:26657"
    let rpc: &str = &std::env::var("RPC").expect("RPC must be set");
    let url = Url::from_str(rpc).expect("Invalid RPC address");

    let app_state = Arc::new(AppState::new(url).await);

    let app: Router = Router::new()
        .route("/cosmos/staking/v1beta1/validators/:address", get(validators_handler))
        .with_state(app_state)
        .layer(cors);

    println!("Starting server...");
    axum::serve(listener, app).await.unwrap();
}

pub async fn validators_handler(Path(address): Path<String>, State(app_state): State<Arc<AppState>>) -> impl IntoResponse {
    let validator = Address::from_str(&address).unwrap();
    let epoch = rpc::query_epoch(app_state.get_client()).await.unwrap();
    let (metadata, commission_info) = rpc::query_metadata(app_state.get_client(), &validator, None).await.unwrap();
    let state = rpc::get_validator_state(app_state.get_client(), &validator, None).await.unwrap();
    let parsed_state: Option<String>;
    let mut jailed: Option<bool>;
    match state {
        Some(s) => {
            jailed = Some(false);
            match s {
                // change to match cosmos equivalent
                ValidatorState::Consensus => parsed_state = Some("CONSENSUS".to_string()),
                ValidatorState::BelowCapacity => parsed_state = Some("BELOW_CAPACITY".to_string()),
                ValidatorState::BelowThreshold => parsed_state = Some("BELOW_THRESHOLD".to_string()),
                ValidatorState::Inactive => parsed_state = Some("INACTIVE".to_string()),
                ValidatorState::Jailed => {
                    parsed_state = Some("JAILED".to_string());
                    jailed = Some(true);
                }
            }
        }
        None => {
            parsed_state = None;
            jailed = None;
        }
    }
    let bonded_total = rpc::get_validator_stake(app_state.get_client(), epoch, &validator).await.unwrap();

    let json_response = serde_json::json!({
        "validator": {
            "operator_address": validator.to_string(),
            "jailed": jailed,
            "status": parsed_state,
            "tokens": bonded_total.to_string_native(),
            "description": {
                "moniker": metadata.as_ref().unwrap().discord_handle,
                "identity": null,
                "website": metadata.as_ref().unwrap().website,
                "security_contact": metadata.as_ref().unwrap().email,
                "details": metadata.as_ref().unwrap().description,
            },
            "commission": {
                "commission_rates": {
                    "rate": commission_info.as_ref().unwrap().commission_rate.to_string(),
                    "max_rate": null,
                    "max_change_rate": commission_info.as_ref().unwrap().max_commission_change_per_epoch.to_string(),
                },
                "update_time": null
            },
        }
    });
    Json(json_response)
}