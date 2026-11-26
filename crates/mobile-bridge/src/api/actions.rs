//! Actions on a whole conversation, sent to the in-process engine as the
//! same journaled plan the desktop sends: the local change is immediate and
//! the provider is updated by the engine's action worker, retried if the
//! network is down.

use service_api::{
    ActionPlanAck, ActionWireOperation, ActionWirePlan, OperationId, PlanId, RequestParams,
    WireMailOperation,
};

fn run_on_thread(
    account_id: String,
    thread_id: String,
    operation: WireMailOperation,
) -> Result<(), String> {
    let plan = ActionWirePlan {
        plan_id: PlanId::new_v7(),
        operations: vec![ActionWireOperation {
            operation_id: OperationId(0),
            account_id,
            thread_id,
            operation,
        }],
    };
    let ack: ActionPlanAck = super::engine::call(RequestParams::ActionExecutePlan { plan })?;
    if !ack.journaled {
        return Err("the engine refused the action".to_owned());
    }
    Ok(())
}

/// Move the conversation out of the inbox into the archive.
pub fn archive_thread(account_id: String, thread_id: String) -> Result<(), String> {
    run_on_thread(account_id, thread_id, WireMailOperation::Archive)
}

/// Move the conversation to the trash.
pub fn trash_thread(account_id: String, thread_id: String) -> Result<(), String> {
    run_on_thread(account_id, thread_id, WireMailOperation::Trash)
}
