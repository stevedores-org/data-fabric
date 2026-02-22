use serde::Serialize;
use worker::*;

#[derive(Serialize)]
struct HealthResponse<'a> {
    service: &'a str,
    status: &'a str,
    mission: &'a str,
}

#[event(fetch)]
pub async fn fetch(req: Request, _env: Env, _ctx: Context) -> Result<Response> {
    console_error_panic_hook::set_once();

    let path = req.path();
    match (req.method(), path.as_str()) {
        (Method::Get, "/") => Response::ok("data-fabric-worker online"),
        (Method::Get, "/health") => Response::from_json(&HealthResponse {
            service: "data-fabric",
            status: "ok",
            mission: "velocity-for-autonomous-agent-builders",
        }),
        _ => Response::error("not found", 404),
    }
}
