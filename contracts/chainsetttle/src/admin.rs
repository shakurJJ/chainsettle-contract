use soroban_sdk::{contractimpl, token, Address, BytesN, Env, String, Symbol, Vec};

use crate::{
    constants::{AUDIT_LOG_MAX_ENTRIES, MAX_FEE_BPS},
    storage,
    AuditEntry, ChainSettleContract, ContractStats, DisputeEntry, FeeConfig, MultiAdminConfig,
};

#[contractimpl]
impl ChainSettleContract {
    // ----------------------------------------------------------
    // INIT
    // ----------------------------------------------------------

    pub fn init(env: Env, admin: Address) {
        admin.require_auth();
        storage::set_admin(&env, &admin);
        storage::set_paused(&env, false);
        storage::set_min_milestone_percent(&env, 5u32);
        storage::set_max_concurrent_disputes(&env, 1u32);
        storage::set_admin_action_log(&env, &Vec::new(&env));
        storage::set_contract_stats(
            &env,
            &ContractStats {
                total_shipments: 0,
                total_volume: 0,
                total_disputes: 0,
                completed_shipments: 0,
            },
        );
        storage::set_active_disputes(&env, &Vec::<DisputeEntry>::new(&env));
        storage::set_escalation_threshold(&env, 0u32);
        storage::set_max_shipment_value(&env, 0i128);
        storage::set_circuit_breaker_limit(&env, 0i128);
        storage::set_circuit_breaker_window(&env, 0u32);
        storage::set_circuit_breaker_window_start(&env, 0u32);
        storage::set_circuit_breaker_window_outflow(&env, 0i128);
    }

    // ----------------------------------------------------------
    // UPGRADE
    // ----------------------------------------------------------

    pub fn upgrade(env: Env, admin: Address, new_wasm_hash: BytesN<32>) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        env.deployer().update_current_contract_wasm(new_wasm_hash.clone());
        env.events().publish(
            (Symbol::new(&env, "contract_upgraded"),),
            (new_wasm_hash, env.ledger().sequence()),
        );
    }

    pub fn migrate(_env: Env) {
        // No-op for current version; implement data migrations here post-upgrade.
    }

    // ----------------------------------------------------------
    // PAUSE / UNPAUSE
    // ----------------------------------------------------------

    pub fn pause(env: Env, admin: Address) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        Self::append_admin_action(&env, Symbol::new(&env, "pause"), Symbol::new(&env, "contract_paused"));
        storage::set_paused(&env, true);
        env.events().publish(
            (Symbol::new(&env, "contract_paused"),),
            env.ledger().sequence(),
        );
    }

    pub fn unpause(env: Env, admin: Address) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        Self::append_admin_action(&env, Symbol::new(&env, "unpause"), Symbol::new(&env, "contract_unpaused"));
        storage::set_paused(&env, false);
        env.events().publish(
            (Symbol::new(&env, "contract_unpaused"),),
            env.ledger().sequence(),
        );
    }

    // ----------------------------------------------------------
    // ESCALATION THRESHOLD
    // ----------------------------------------------------------

    pub fn set_escalation_threshold(env: Env, admin: Address, threshold_ledgers: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        storage::set_escalation_threshold(&env, threshold_ledgers);
        env.events().publish(
            (Symbol::new(&env, "escalation_threshold_set"),),
            threshold_ledgers,
        );
    }

    pub fn get_escalation_threshold(env: Env) -> u32 {
        storage::get_escalation_threshold(&env)
    }

    // ----------------------------------------------------------
    // MAX SHIPMENT VALUE
    // ----------------------------------------------------------

    pub fn set_max_shipment_value(env: Env, admin: Address, max_value: i128) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        storage::set_max_shipment_value(&env, max_value);
        env.events().publish(
            (Symbol::new(&env, "max_shipment_value_set"),),
            max_value,
        );
    }

    pub fn get_max_shipment_value(env: Env) -> i128 {
        storage::get_max_shipment_value(&env)
    }

    // ----------------------------------------------------------
    // CIRCUIT BREAKER
    // ----------------------------------------------------------

    pub fn set_circuit_breaker(env: Env, admin: Address, limit: i128, window_ledgers: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        storage::set_circuit_breaker_limit(&env, limit);
        storage::set_circuit_breaker_window(&env, window_ledgers);
        storage::set_circuit_breaker_window_start(&env, env.ledger().sequence());
        storage::set_circuit_breaker_window_outflow(&env, 0i128);
        env.events().publish(
            (Symbol::new(&env, "circuit_breaker_set"),),
            (limit, window_ledgers),
        );
    }

    // ----------------------------------------------------------
    // MULTI-ADMIN GOVERNANCE
    // ----------------------------------------------------------

    pub fn initialize_multisig_admin(env: Env, admin: Address, admins: Vec<Address>, threshold: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        if admins.len() < 1 || threshold < 1 || threshold > admins.len() as u32 {
            panic!("invalid multi-sig parameters");
        }
        storage::set_multi_admin_config(&env, &MultiAdminConfig { admins, threshold });
        env.events().publish(
            (Symbol::new(&env, "multisig_admin_initialized"),),
            threshold,
        );
    }

    pub fn propose_admin_action(
        env: Env,
        admin: Address,
        action_id: String,
        operation: Symbol,
        params: String,
    ) {
        admin.require_auth();
        let config = storage::get_multi_admin_config(&env)
            .unwrap_or_else(|| panic!("multisig admin not configured"));

        let mut is_admin = false;
        for i in 0..config.admins.len() {
            if config.admins.get(i).unwrap() == admin {
                is_admin = true;
                break;
            }
        }
        if !is_admin {
            panic!("unauthorized");
        }

        let mut approvals = storage::get_admin_approvals(&env, &action_id);
        for i in 0..approvals.len() {
            if approvals.get(i).unwrap() == admin {
                panic!("already approved by this admin");
            }
        }
        approvals.push_back(admin.clone());
        storage::set_admin_approvals(&env, &action_id, &approvals);

        env.events().publish(
            (Symbol::new(&env, "admin_action_proposed"), action_id.clone()),
            approvals.len() as u32,
        );

        if approvals.len() as u32 >= config.threshold {
            Self::execute_admin_action(&env, &action_id, operation, params);
            storage::remove_admin_approvals(&env, &action_id);
        }
    }

    pub fn get_pending_admin_actions(env: Env, action_id: String) -> Vec<Address> {
        storage::get_admin_approvals(&env, &action_id)
    }

    fn execute_admin_action(env: &Env, action_id: &String, operation: Symbol, _params: String) {
        env.events().publish(
            (Symbol::new(env, "admin_action_executed"), action_id.clone()),
            operation,
        );
    }

    // ----------------------------------------------------------
    // FEE CONFIG
    // ----------------------------------------------------------

    pub fn set_fee_config(env: Env, admin: Address, fee_bps: u32, treasury: Address) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        if fee_bps > MAX_FEE_BPS {
            panic!("fee_bps exceeds maximum of 1000");
        }
        Self::append_admin_action(&env, Symbol::new(&env, "set_fee_config"), Symbol::new(&env, "fee_config_updated"));
        storage::set_fee_config(&env, &FeeConfig { fee_bps, treasury });
    }

    pub fn set_max_concurrent_disputes(env: Env, admin: Address, limit: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        storage::set_max_concurrent_disputes(&env, limit);
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "set_max_concurrent_disputes"),
            Symbol::new(&env, "max_concurrent_disputes_updated"),
        );
    }

    pub fn set_min_milestone_percent(env: Env, admin: Address, percent: u32) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        if percent == 0 || percent > 100 {
            panic!("min_milestone_percent must be between 1 and 100");
        }
        storage::set_min_milestone_percent(&env, percent);
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "set_min_milestone_percent"),
            Symbol::new(&env, "min_milestone_percent_updated"),
        );
    }

    pub fn blacklist_address(env: Env, admin: Address, address: Address, reason_hash: BytesN<32>) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        storage::set_blacklisted(&env, &address, &reason_hash);
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "blacklist_address"),
            Symbol::new(&env, "address_blacklisted"),
        );
    }

    pub fn remove_from_blacklist(env: Env, admin: Address, address: Address) {
        admin.require_auth();
        Self::assert_admin(&env, &admin);
        storage::remove_blacklisted(&env, &address);
        Self::append_admin_action(
            &env,
            Symbol::new(&env, "remove_from_blacklist"),
            Symbol::new(&env, "address_unblacklisted"),
        );
    }

    pub fn is_blacklisted(env: Env, address: Address) -> bool {
        storage::is_blacklisted(&env, &address)
    }

    pub fn get_admin_log(env: Env) -> Vec<AuditEntry> {
        storage::get_admin_action_log(&env)
    }

    // ----------------------------------------------------------
    // TOKEN WHITELIST
    // ----------------------------------------------------------

    pub fn add_allowed_token(env: Env, token: Address) {
        let admin = storage::get_admin(&env).unwrap_or_else(|| panic!("unauthorized"));
        admin.require_auth();
        Self::append_admin_action(&env, Symbol::new(&env, "add_allowed_token"), Symbol::new(&env, "allowed_token_added"));
        let mut allowed = storage::get_allowed_tokens(&env);
        allowed.push_back(token);
        storage::set_allowed_tokens(&env, &allowed);
    }

    pub fn remove_allowed_token(env: Env, token: Address) {
        let admin = storage::get_admin(&env).unwrap_or_else(|| panic!("unauthorized"));
        admin.require_auth();
        Self::append_admin_action(&env, Symbol::new(&env, "remove_allowed_token"), Symbol::new(&env, "allowed_token_removed"));
        let allowed = storage::get_allowed_tokens(&env);
        let mut new_list: Vec<Address> = Vec::new(&env);
        for i in 0..allowed.len() {
            let t = allowed.get(i).unwrap();
            if t != token {
                new_list.push_back(t);
            }
        }
        storage::set_allowed_tokens(&env, &new_list);
    }

    // ----------------------------------------------------------
    // TWO-STEP ADMIN TRANSFER
    // ----------------------------------------------------------

    pub fn nominate_admin(env: Env, current_admin: Address, nominee: Address) {
        current_admin.require_auth();
        Self::assert_admin(&env, &current_admin);
        storage::set_pending_admin(&env, &nominee);
        env.events().publish(
            (Symbol::new(&env, "admin_nominated"),),
            nominee,
        );
    }

    pub fn accept_admin(env: Env, nominee: Address) {
        nominee.require_auth();
        let pending = storage::get_pending_admin(&env)
            .unwrap_or_else(|| panic!("no pending nomination"));
        if nominee != pending {
            panic!("unauthorized");
        }
        let old_admin = storage::get_admin(&env).unwrap_or_else(|| panic!("unauthorized"));
        storage::set_admin(&env, &nominee);
        storage::remove_pending_admin(&env);
        env.events().publish(
            (Symbol::new(&env, "admin_transferred"),),
            (old_admin, nominee),
        );
    }

    pub fn revoke_nomination(env: Env, current_admin: Address) {
        current_admin.require_auth();
        Self::assert_admin(&env, &current_admin);
        storage::remove_pending_admin(&env);
        env.events().publish(
            (Symbol::new(&env, "nomination_revoked"),),
            env.ledger().sequence(),
        );
    }

    // ----------------------------------------------------------
    // EMERGENCY FUND RECOVERY
    // ----------------------------------------------------------

    pub fn emergency_recover(env: Env, admin: Address, shipment_id: String) {
        Self::assert_not_paused(&env);
        admin.require_auth();
        Self::assert_admin(&env, &admin);

        let mut shipment = Self::get_shipment_internal(&env, &shipment_id);

        if shipment.status != crate::ShipmentStatus::Active {
            panic!("shipment is not active");
        }

        let current_ledger = env.ledger().sequence();
        if current_ledger <= shipment.created_at + crate::constants::RECOVERY_THRESHOLD_LEDGERS {
            panic!("recovery threshold not reached");
        }

        let recovery_amount = shipment.total_amount - shipment.released_amount;

        if recovery_amount > 0 {
            let token_client = token::Client::new(&env, &shipment.token);
            token_client.transfer(&env.current_contract_address(), &admin, &recovery_amount);
        }

        Self::move_shipment_status_index(
            &env,
            crate::ShipmentStatus::Active,
            crate::ShipmentStatus::Cancelled,
            &shipment_id,
        );
        shipment.status = crate::ShipmentStatus::Cancelled;
        storage::set_shipment(&env, &shipment_id, &shipment);

        let escrowed = storage::get_total_escrowed(&env, &shipment.token);
        storage::set_total_escrowed(&env, &shipment.token, (escrowed - recovery_amount).max(0));

        env.events().publish(
            (Symbol::new(&env, "emergency_recovery"), shipment_id.clone()),
            (recovery_amount, admin),
        );
    }

    // ----------------------------------------------------------
    // INTERNAL HELPERS (shared across modules)
    // ----------------------------------------------------------

    pub(crate) fn assert_not_paused(env: &Env) {
        if storage::is_paused(env) {
            panic!("contract is paused");
        }
    }

    pub(crate) fn assert_admin(env: &Env, caller: &Address) {
        let stored = storage::get_admin(env).unwrap_or_else(|| panic!("unauthorized"));
        if *caller != stored {
            panic!("unauthorized");
        }
    }

    pub(crate) fn is_buyer(shipment: &crate::Shipment, addr: &Address) -> bool {
        for i in 0..shipment.buyers.len() {
            if shipment.buyers.get(i).unwrap() == *addr {
                return true;
            }
        }
        false
    }

    pub(crate) fn assert_is_buyer(shipment: &crate::Shipment, addr: &Address) {
        if !Self::is_buyer(shipment, addr) {
            panic!("unauthorized");
        }
    }

    pub(crate) fn assert_no_open_disputes(shipment: &crate::Shipment) {
        for i in 0..shipment.milestones.len() {
            if shipment.milestones.get(i).unwrap().status == crate::MilestoneStatus::Disputed {
                panic!("transfer disallowed: open dispute must be resolved first");
            }
        }
    }

    pub(crate) fn check_circuit_breaker(env: &Env, payment: i128) {
        let limit = storage::get_circuit_breaker_limit(env);
        if limit == 0 {
            return;
        }
        let window = storage::get_circuit_breaker_window(env);
        let window_start = storage::get_circuit_breaker_window_start(env);
        let mut outflow = storage::get_circuit_breaker_window_outflow(env);
        let current_ledger = env.ledger().sequence();

        if current_ledger >= window_start + window {
            outflow = 0;
            storage::set_circuit_breaker_window_start(env, current_ledger);
        }
        if outflow + payment > limit {
            panic!("circuit breaker triggered: outflow limit exceeded");
        }
        storage::set_circuit_breaker_window_outflow(env, outflow + payment);
    }

    pub(crate) fn get_shipment_internal(env: &Env, shipment_id: &String) -> crate::Shipment {
        storage::get_shipment(env, shipment_id)
            .unwrap_or_else(|| panic!("shipment not found"))
    }

    pub(crate) fn append_audit_entry(
        env: &Env,
        shipment: &mut crate::Shipment,
        action: Symbol,
        detail: Symbol,
    ) {
        let entry = AuditEntry {
            action,
            caller: storage::get_admin(env).unwrap_or_else(|| panic!("unauthorized")),
            ledger: env.ledger().sequence(),
            detail,
        };
        if shipment.audit_log.len() as usize >= crate::constants::SHIPMENT_AUDIT_LOG_MAX_ENTRIES {
            let mut new_log: Vec<AuditEntry> = Vec::new(env);
            for i in 1..shipment.audit_log.len() {
                new_log.push_back(shipment.audit_log.get(i).unwrap());
            }
            shipment.audit_log = new_log;
        }
        shipment.audit_log.push_back(entry);
    }

    pub(crate) fn append_admin_action(env: &Env, action: Symbol, detail: Symbol) {
        let mut log = storage::get_admin_action_log(env);
        let entry = AuditEntry {
            action,
            caller: storage::get_admin(env).unwrap_or_else(|| panic!("unauthorized")),
            ledger: env.ledger().sequence(),
            detail,
        };
        if log.len() as usize >= AUDIT_LOG_MAX_ENTRIES {
            let mut next: Vec<AuditEntry> = Vec::new(env);
            for i in 1..log.len() {
                next.push_back(log.get(i).unwrap());
            }
            log = next;
        }
        log.push_back(entry);
        storage::set_admin_action_log(env, &log);
    }

    pub(crate) fn all_milestones_done(shipment: &crate::Shipment) -> bool {
        for i in 0..shipment.milestones.len() {
            let s = shipment.milestones.get(i).unwrap().status;
            if s != crate::MilestoneStatus::Confirmed && s != crate::MilestoneStatus::Resolved {
                return false;
            }
        }
        true
    }

    pub(crate) fn deduct_fee(env: &Env, gross: i128, token: &Address, fee_out: &mut i128) -> i128 {
        if let Some(config) = storage::get_fee_config(env) {
            let fee = (gross * config.fee_bps as i128) / 10_000;
            if fee > 0 {
                let token_client = token::Client::new(env, token);
                token_client.transfer(&env.current_contract_address(), &config.treasury, &fee);
                *fee_out = fee;
                return gross - fee;
            }
        }
        gross
    }

    pub(crate) fn add_to_status_index(env: &Env, status: crate::ShipmentStatus, shipment_id: &String) {
        let mut list = storage::get_shipments_by_status(env, status.clone());
        list.push_back(shipment_id.clone());
        storage::set_shipments_by_status(env, status, &list);
    }

    pub(crate) fn remove_from_status_index(
        env: &Env,
        status: crate::ShipmentStatus,
        shipment_id: &String,
    ) {
        let list = storage::get_shipments_by_status(env, status.clone());
        let mut new_list: Vec<String> = Vec::new(env);
        let mut removed = false;
        for i in 0..list.len() {
            let id = list.get(i).unwrap();
            if !removed && id == *shipment_id {
                removed = true;
            } else {
                new_list.push_back(id);
            }
        }
        storage::set_shipments_by_status(env, status, &new_list);
    }

    pub(crate) fn move_shipment_status_index(
        env: &Env,
        from: crate::ShipmentStatus,
        to: crate::ShipmentStatus,
        shipment_id: &String,
    ) {
        Self::remove_from_status_index(env, from, shipment_id);
        Self::add_to_status_index(env, to, shipment_id);
    }
}
