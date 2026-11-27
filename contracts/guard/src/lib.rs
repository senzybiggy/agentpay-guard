#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, Address, Env, String,
};

const TTL_THRESHOLD: u32 = 100_000;
const TTL_EXTEND_TO: u32 = 200_000;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Policy {
    pub per_tx_limit: i128,
    pub daily_limit: i128,
    pub auto_approve_limit: i128,
    pub approval_ttl_secs: u64,
}

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

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RejectReason {
    None,
    NotAllowlisted,
    PerTxLimitExceeded,
    DailyLimitExceeded,
    OwnerRejected,
}

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
    pub counted_day: u64,
    pub counted_amount: i128,
    pub attested_not_executed: bool,
    pub tx_hash: Option<String>,
}

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

    fn require_agent(env: &Env, agent: &Address) -> Result<(), Error> {
        let stored_agent: Address = env
            .storage()
            .instance()
            .get(&DataKey::Agent)
            .ok_or(Error::NotInitialized)?;

        if *agent != stored_agent {
            return Err(Error::Unauthorized);
        }

        agent.require_auth();
        Ok(())
    }

    fn save_request(env: &Env, request: &PaymentRequest) {
        let key = DataKey::Request(request.id);
        env.storage().persistent().set(&key, request);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);
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

        if agent == owner {
            return Err(Error::Unauthorized);
        }

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

        if new_agent == owner {
            return Err(Error::Unauthorized);
        }

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
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);

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

    pub fn submit_request(
        env: Env,
        agent: Address,
        recipient: Address,
        amount: i128,
        reason: String,
    ) -> Result<u64, Error> {
        Self::require_agent(&env, &agent)?;

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        if reason.len() > 128 {
            return Err(Error::ReasonTooLong);
        }

        let policy: Policy = env
            .storage()
            .instance()
            .get(&DataKey::Policy)
            .ok_or(Error::NotInitialized)?;

        let request_id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::NextId)
            .ok_or(Error::NotInitialized)?;

        let now = env.ledger().timestamp();
        let today = now / 86_400;

        let spent: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::SpentOnDay(today))
            .unwrap_or(0);

        let reject_reason = if !env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::Allowlist(recipient.clone()))
            .unwrap_or(false)
        {
            RejectReason::NotAllowlisted
        } else if amount > policy.per_tx_limit {
            RejectReason::PerTxLimitExceeded
        } else if spent
            .checked_add(amount)
            .map_or(true, |total| total > policy.daily_limit)
        {
            RejectReason::DailyLimitExceeded
        } else {
            RejectReason::None
        };

        let status = if reject_reason != RejectReason::None {
            Status::Rejected
        } else if amount <= policy.auto_approve_limit {
            Status::AutoApproved
        } else {
            Status::Pending
        };

        let counted_amount = if status == Status::AutoApproved {
            let new_spent = spent.checked_add(amount).ok_or(Error::InvalidAmount)?;
            let key = DataKey::SpentOnDay(today);

            env.storage().persistent().set(&key, &new_spent);
            env.storage()
                .persistent()
                .extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);

            amount
        } else {
            0
        };

        let request = PaymentRequest {
            id: request_id,
            agent: agent.clone(),
            recipient: recipient.clone(),
            amount,
            reason,
            status: status.clone(),
            reject_reason: reject_reason.clone(),
            created_at: now,
            expires_at: now.saturating_add(policy.approval_ttl_secs),
            counted_day: if counted_amount > 0 { today } else { 0 },
            counted_amount,
            attested_not_executed: false,
            tx_hash: None,
        };

        Self::save_request(&env, &request);

        env.storage()
            .instance()
            .set(&DataKey::NextId, &(request_id + 1));

        RequestSubmitted {
            request_id,
            agent,
            recipient,
            amount,
            status: status.clone(),
        }
        .publish(&env);

        if reject_reason != RejectReason::None {
            RequestRejected {
                request_id,
                reason: reject_reason,
                amount,
            }
            .publish(&env);
        }

        Ok(request_id)
    }

    pub fn approve_request(
        env: Env,
        owner: Address,
        request_id: u64,
    ) -> Result<Status, Error> {
        Self::require_owner(&env, &owner)?;

        let mut request: PaymentRequest = env
            .storage()
            .persistent()
            .get(&DataKey::Request(request_id))
            .ok_or(Error::RequestNotFound)?;

        if request.status != Status::Pending {
            return Err(Error::InvalidStatus);
        }

        let now = env.ledger().timestamp();

        if now >= request.expires_at {
            return Err(Error::RequestExpired);
        }

        let policy: Policy = env
            .storage()
            .instance()
            .get(&DataKey::Policy)
            .ok_or(Error::NotInitialized)?;

        let today = now / 86_400;

        let spent_today: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::SpentOnDay(today))
            .unwrap_or(0);

        let allowed: bool = env
            .storage()
            .persistent()
            .get(&DataKey::Allowlist(request.recipient.clone()))
            .unwrap_or(false);

        let reject_reason = if !allowed {
            Some(RejectReason::NotAllowlisted)
        } else if request.amount > policy.per_tx_limit {
            Some(RejectReason::PerTxLimitExceeded)
        } else if spent_today
            .checked_add(request.amount)
            .map_or(true, |total| total > policy.daily_limit)
        {
            Some(RejectReason::DailyLimitExceeded)
        } else {
            None
        };

        if let Some(reason) = reject_reason {
            request.status = Status::Rejected;
            request.reject_reason = reason.clone();

            Self::save_request(&env, &request);

            RequestRejected {
                request_id,
                reason,
                amount: request.amount,
            }
            .publish(&env);

            return Ok(Status::Rejected);
        }

        let new_spent = spent_today
            .checked_add(request.amount)
            .ok_or(Error::InvalidAmount)?;

        let spent_key = DataKey::SpentOnDay(today);

        env.storage()
            .persistent()
            .set(&spent_key, &new_spent);

        env.storage()
            .persistent()
            .extend_ttl(&spent_key, TTL_THRESHOLD, TTL_EXTEND_TO);

        request.status = Status::Approved;
        request.counted_day = today;
        request.counted_amount = request.amount;

        Self::save_request(&env, &request);

        RequestApproved {
            request_id,
            owner,
            amount: request.amount,
        }
        .publish(&env);

        Ok(Status::Approved)
    }

    pub fn reject_request(
        env: Env,
        owner: Address,
        request_id: u64,
    ) -> Result<Status, Error> {
        Self::require_owner(&env, &owner)?;

        let mut request: PaymentRequest = env
            .storage()
            .persistent()
            .get(&DataKey::Request(request_id))
            .ok_or(Error::RequestNotFound)?;

        if request.status != Status::Pending {
            return Err(Error::InvalidStatus);
        }

        request.status = Status::Rejected;
        request.reject_reason = RejectReason::OwnerRejected;

        Self::save_request(&env, &request);

        RequestRejected {
            request_id,
            reason: RejectReason::OwnerRejected,
            amount: request.amount,
        }
        .publish(&env);

        Ok(Status::Rejected)
    }

    // STAGE 6: CANCELLATION, EXPIRY AND EXECUTION

    pub fn cancel_approved(
        env: Env,
        owner: Address,
        request_id: u64,
        not_executed: bool,
    ) -> Result<Status, Error> {
        Self::require_owner(&env, &owner)?;

        if !not_executed {
            return Err(Error::AttestationRequired);
        }

        let mut request: PaymentRequest = env
            .storage()
            .persistent()
            .get(&DataKey::Request(request_id))
            .ok_or(Error::RequestNotFound)?;

        if request.status != Status::Approved
            && request.status != Status::AutoApproved
        {
            return Err(Error::InvalidStatus);
        }

        let released_amount = request.counted_amount;

        if released_amount > 0 {
            let spent_key = DataKey::SpentOnDay(request.counted_day);

            let spent: i128 = env
                .storage()
                .persistent()
                .get(&spent_key)
                .unwrap_or(0);

            let remaining = spent
                .checked_sub(released_amount)
                .ok_or(Error::InvalidAmount)?;

            env.storage()
                .persistent()
                .set(&spent_key, &remaining);

            env.storage()
                .persistent()
                .extend_ttl(&spent_key, TTL_THRESHOLD, TTL_EXTEND_TO);
        }

        request.status = Status::Cancelled;
        request.attested_not_executed = true;

        Self::save_request(&env, &request);

        RequestCancelled {
            request_id,
            owner,
            attested_not_executed: true,
            released_amount,
        }
        .publish(&env);

        Ok(Status::Cancelled)
    }

    pub fn expire_request(
        env: Env,
        actor: Address,
        request_id: u64,
    ) -> Result<Status, Error> {
        let owner = Self::stored_owner(&env)?;

        let agent: Address = env
            .storage()
            .instance()
            .get(&DataKey::Agent)
            .ok_or(Error::NotInitialized)?;

        if actor != owner && actor != agent {
            return Err(Error::Unauthorized);
        }

        actor.require_auth();

        let mut request: PaymentRequest = env
            .storage()
            .persistent()
            .get(&DataKey::Request(request_id))
            .ok_or(Error::RequestNotFound)?;

        if request.status != Status::Pending {
            return Err(Error::InvalidStatus);
        }

        let now = env.ledger().timestamp();

        if now < request.expires_at {
            return Err(Error::InvalidStatus);
        }

        request.status = Status::Expired;

        Self::save_request(&env, &request);

        RequestExpired {
            request_id,
            previous_status: Status::Pending,
        }
        .publish(&env);

        Ok(Status::Expired)
    }

    pub fn mark_executed(
        env: Env,
        owner: Address,
        request_id: u64,
        tx_hash: String,
    ) -> Result<Status, Error> {
        Self::require_owner(&env, &owner)?;

        if tx_hash.len() == 0 || tx_hash.len() > 128 {
            return Err(Error::InvalidTxHash);
        }

        let mut request: PaymentRequest = env
            .storage()
            .persistent()
            .get(&DataKey::Request(request_id))
            .ok_or(Error::RequestNotFound)?;

        if request.status != Status::Approved
            && request.status != Status::AutoApproved
        {
            return Err(Error::InvalidStatus);
        }

        request.status = Status::Executed;
        request.tx_hash = Some(tx_hash.clone());

        Self::save_request(&env, &request);

        RequestExecuted {
            request_id,
            amount: request.amount,
            tx_hash,
        }
        .publish(&env);

        Ok(Status::Executed)
    }
    // STAGE 7: READ-ONLY VIEW FUNCTIONS

    pub fn get_policy(env: Env) -> Result<Policy, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Policy)
            .ok_or(Error::NotInitialized)
    }

    pub fn get_request(
        env: Env,
        request_id: u64,
    ) -> Result<PaymentRequest, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Request(request_id))
            .ok_or(Error::RequestNotFound)
    }

    pub fn request_count(env: Env) -> Result<u64, Error> {
        let next_id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::NextId)
            .ok_or(Error::NotInitialized)?;

        Ok(next_id.saturating_sub(1))
    }

    pub fn spent_today(env: Env) -> Result<i128, Error> {
        let today = env.ledger().timestamp() / 86_400;

        Ok(env
            .storage()
            .persistent()
            .get(&DataKey::SpentOnDay(today))
            .unwrap_or(0))
    }

    pub fn is_allowed(
        env: Env,
        recipient: Address,
    ) -> Result<bool, Error> {
        Ok(env
            .storage()
            .persistent()
            .get(&DataKey::Allowlist(recipient))
            .unwrap_or(false))
    }
}

#[cfg(test)]
mod test;