#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, Address, Env, String,
};

/// Spending policy set by the owner. Owner and agent addresses are stored
/// separately (see DataKey::Owner / DataKey::Agent); this struct holds
/// only the limits and the approval time-to-live.
/// All amounts are in stroops (1 XLM = 10_000_000 stroops).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Policy {
    pub per_tx_limit: i128,
    pub daily_limit: i128,
    pub auto_approve_limit: i128,
    pub approval_ttl_secs: u64,
}

/// Lifecycle of a payment request. Terminal states (no further
/// transitions allowed): Rejected, Cancelled, Expired, Executed.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Status {
    Pending,
    AutoApproved,
    Approved,
    Rejected,
    Cancelled,
    Expired,
    Executed,
}

/// Machine-readable reason stored on a request when it is rejected.
/// The frontend maps these to human-readable text.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RejectReason {
    None,
    NotAllowlisted,
    PerTxLimitExceeded,
    DailyLimitExceeded,
    OwnerRejected,
}

/// A single agent payment request and its full decision record.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaymentRequest {
    pub id: u64,
    pub agent: Address,
    pub recipient: Address,
    pub amount: i128,
    pub reason: String,
    pub status: Status,
    pub reject_reason: RejectReason,
    pub created_at: u64,
    pub expires_at: u64,
    /// Day bucket (ledger timestamp / 86400) that this request's
    /// reservation was counted against. 0 if never counted.
    pub counted_day: u64,
    /// Amount reserved against counted_day. 0 if never counted.
    pub counted_amount: i128,
    /// Set when an approved request is cancelled with the owner's
    /// attestation that the payment was not executed.
    pub attested_not_executed: bool,
    /// Classic Stellar transaction hash, set by mark_executed.
    pub tx_hash: Option<String>,
}

/// Storage keys. Owner, Agent, Policy and NextId live in instance
/// storage; Request, Allowlist and SpentOnDay live in persistent storage.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Owner,
    Agent,
    Policy,
    NextId,
    Request(u64),
    Allowlist(Address),
    SpentOnDay(u64),
}

/// Contract errors. Malformed or invalid operations return these;
/// policy violations do NOT — they are recorded as rejected requests.
/// Numbering is frozen once published: do not renumber.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    Unauthorized = 2,
    InvalidPolicy = 3,
    InvalidAmount = 4,
    ReasonTooLong = 5,
    RequestNotFound = 6,
    InvalidStatus = 7,
    AttestationRequired = 8,
    InvalidTxHash = 9,
    RequestExpired = 10,
}

// ---------------------------------------------------------------------------
// Events (Stage 2)
//
// Events are the live activity feed only. Stored PaymentRequest records
// remain the source of truth for history, because Soroban RPC event
// retention is limited. Fields marked #[topic] are indexed topics;
// all other fields are published in the event data map.
// ---------------------------------------------------------------------------

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicySet {
    #[topic]
    pub owner: Address,
    pub per_tx_limit: i128,
    pub daily_limit: i128,
    pub auto_approve_limit: i128,
    pub approval_ttl_secs: u64,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentSet {
    #[topic]
    pub agent: Address,
    pub owner: Address,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecipientAdded {
    #[topic]
    pub recipient: Address,
    pub owner: Address,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecipientRemoved {
    #[topic]
    pub recipient: Address,
    pub owner: Address,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestSubmitted {
    #[topic]
    pub request_id: u64,
    #[topic]
    pub agent: Address,
    pub recipient: Address,
    pub amount: i128,
    pub status: Status,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestApproved {
    #[topic]
    pub request_id: u64,
    pub owner: Address,
    pub amount: i128,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestRejected {
    #[topic]
    pub request_id: u64,
    pub reason: RejectReason,
    pub amount: i128,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestCancelled {
    #[topic]
    pub request_id: u64,
    pub owner: Address,
    pub attested_not_executed: bool,
    pub released_amount: i128,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestExpired {
    #[topic]
    pub request_id: u64,
    pub previous_status: Status,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestExecuted {
    #[topic]
    pub request_id: u64,
    pub amount: i128,
    pub tx_hash: String,
}

#[contract]
pub struct GuardContract;
impl GuardContract {
    fn validate_policy(
        per_tx_limit: i128,
        daily_limit: i128,
        auto_approve_limit: i128,
        approval_ttl_secs: u64,
    ) -> Result<(), Error> {
        if per_tx_limit <= 0
            || daily_limit <= 0
            || auto_approve_limit <= 0
            || auto_approve_limit > per_tx_limit
            || per_tx_limit > daily_limit
            || approval_ttl_secs == 0
        {
            return Err(Error::InvalidPolicy);
        }
        Ok(())
    }

    fn stored_owner(env: &Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Owner)
            .ok_or(Error::NotInitialized)
    }

    fn require_owner(env: &Env, owner: &Address) -> Result<(), Error> {
        if *owner != Self::stored_owner(env)? {
            return Err(Error::Unauthorized);
        }
        owner.require_auth();
        Ok(())
    }
}

#[contractimpl]
impl GuardContract {
    pub fn __constructor(
        env: Env,
        owner: Address,
        agent: Address,
        per_tx_limit: i128,
        daily_limit: i128,
        auto_approve_limit: i128,
        approval_ttl_secs: u64,
    ) -> Result<(), Error> {
        owner.require_auth();

        Self::validate_policy(
            per_tx_limit,
            daily_limit,
            auto_approve_limit,
            approval_ttl_secs,
        )?;

        let policy = Policy {
            per_tx_limit,
            daily_limit,
            auto_approve_limit,
            approval_ttl_secs,
        };

        env.storage().instance().set(&DataKey::Owner, &owner);
        env.storage().instance().set(&DataKey::Agent, &agent);
        env.storage().instance().set(&DataKey::Policy, &policy);
        env.storage().instance().set(&DataKey::NextId, &1u64);

        PolicySet {
            owner,
            per_tx_limit,
            daily_limit,
            auto_approve_limit,
            approval_ttl_secs,
        }
        .publish(&env);

        Ok(())
    }

    pub fn set_policy(
        env: Env,
        owner: Address,
        per_tx_limit: i128,
        daily_limit: i128,
        auto_approve_limit: i128,
        approval_ttl_secs: u64,
    ) -> Result<(), Error> {
        Self::require_owner(&env, &owner)?;

        Self::validate_policy(
            per_tx_limit,
            daily_limit,
            auto_approve_limit,
            approval_ttl_secs,
        )?;

        let policy = Policy {
            per_tx_limit,
            daily_limit,
            auto_approve_limit,
            approval_ttl_secs,
        };

        env.storage().instance().set(&DataKey::Policy, &policy);

        PolicySet {
            owner,
            per_tx_limit,
            daily_limit,
            auto_approve_limit,
            approval_ttl_secs,
        }
        .publish(&env);

        Ok(())
    }

    pub fn set_agent(
        env: Env,
        owner: Address,
        new_agent: Address,
    ) -> Result<(), Error> {
        Self::require_owner(&env, &owner)?;

        env.storage().instance().set(&DataKey::Agent, &new_agent);

        AgentSet {
            agent: new_agent,
            owner,
        }
        .publish(&env);

        Ok(())
    }

    pub fn add_recipient(
        env: Env,
        owner: Address,
        recipient: Address,
    ) -> Result<(), Error> {
        Self::require_owner(&env, &owner)?;

        let key = DataKey::Allowlist(recipient.clone());

        env.storage().persistent().set(&key, &true);
        env.storage().persistent().extend_ttl(
            &key,
            100_000,
            200_000,
        );

        RecipientAdded { recipient, owner }.publish(&env);

        Ok(())
    }

    pub fn remove_recipient(
        env: Env,
        owner: Address,
        recipient: Address,
    ) -> Result<(), Error> {
        Self::require_owner(&env, &owner)?;

        env.storage()
            .persistent()
            .remove(&DataKey::Allowlist(recipient.clone()));

        RecipientRemoved { recipient, owner }.publish(&env);

        Ok(())
    }
}
#[cfg(test)]
mod test;