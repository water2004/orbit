use crate::{
    discovery::{self, Plan},
    transaction::{self, Result},
};

pub fn stage(plan: &Plan) -> Result<()> {
    let leases = discovery::leases(plan)?;
    // Detach instances while their lifecycle leases are held, then stage
    // containing repositories/AppData. Rollback restores parents first.
    transaction::stage(&plan.game_entries)?;
    let result = (|| -> Result<()> {
        for lease in leases {
            lease.release_after_staging()?;
        }
        transaction::stage(&plan.data_entries)
    })();
    if let Err(error) = result {
        let restored = rollback(plan);
        return Err(format!("uninstall staging failed: {error}; rollback: {restored:?}").into());
    }
    Ok(())
}
pub fn rollback(plan: &Plan) -> Result<()> {
    let data = transaction::rollback(&plan.data_entries);
    let games = transaction::rollback(&plan.game_entries);
    if data.is_err() || games.is_err() {
        return Err(format!("data rollback: {data:?}; instance rollback: {games:?}").into());
    }
    Ok(())
}
pub fn commit(plan: &Plan) -> Result<()> {
    transaction::commit(&plan.data_entries)?;
    transaction::commit(&plan.game_entries)?;
    discovery::prune_empty(plan)
}
