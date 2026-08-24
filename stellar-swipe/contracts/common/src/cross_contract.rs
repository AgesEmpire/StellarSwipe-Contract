use core::marker::PhantomData;

use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum CrossContractError {
    InterfaceVersionMismatch = 1,
    InterfaceVersionUnavailable = 2,
}

pub const ORACLE_INTERFACE_VERSION: u32 = 1;
pub const STAKE_VAULT_INTERFACE_VERSION: u32 = 1;

pub trait VersionedContract {
    fn interface_version(&self) -> u32;
}

pub struct VersionedContractClient<T>
where
    T: VersionedContract,
{
    expected_version: u32,
    _marker: PhantomData<T>,
}

impl<T> VersionedContractClient<T>
where
    T: VersionedContract,
{
    pub fn new(expected_version: u32) -> Self {
        Self {
            expected_version,
            _marker: PhantomData,
        }
    }

    pub fn expected_version(&self) -> u32 {
        self.expected_version
    }

    pub fn ensure_compatible(&self, callee: &T) -> Result<u32, CrossContractError> {
        let reported_version = callee.interface_version();
        if reported_version == 0 {
            return Err(CrossContractError::InterfaceVersionUnavailable);
        }
        if reported_version != self.expected_version {
            return Err(CrossContractError::InterfaceVersionMismatch);
        }
        Ok(reported_version)
    }

    pub fn proceed_if_compatible<F, R>(&self, callee: &T, action: F) -> Result<R, CrossContractError>
    where
        F: FnOnce() -> Result<R, CrossContractError>,
    {
        self.ensure_compatible(callee)?;
        action()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OracleCallee {
    interface_version: u32,
}

impl OracleCallee {
    pub fn new(interface_version: u32) -> Self {
        Self { interface_version }
    }
}

impl VersionedContract for OracleCallee {
    fn interface_version(&self) -> u32 {
        self.interface_version
    }
}

pub type OracleClient = VersionedContractClient<OracleCallee>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StakeVaultCallee {
    interface_version: u32,
}

impl StakeVaultCallee {
    pub fn new(interface_version: u32) -> Self {
        Self { interface_version }
    }
}

impl VersionedContract for StakeVaultCallee {
    fn interface_version(&self) -> u32 {
        self.interface_version
    }
}

pub type StakeVaultClient = VersionedContractClient<StakeVaultCallee>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oracle_client_accepts_matching_interface_version() {
        let client = OracleClient::new(ORACLE_INTERFACE_VERSION);
        let callee = OracleCallee::new(ORACLE_INTERFACE_VERSION);

        assert_eq!(client.ensure_compatible(&callee), Ok(ORACLE_INTERFACE_VERSION));
    }

    #[test]
    fn oracle_client_rejects_bumped_interface_version() {
        let client = OracleClient::new(ORACLE_INTERFACE_VERSION);
        let callee = OracleCallee::new(ORACLE_INTERFACE_VERSION + 1);

        assert_eq!(
            client.ensure_compatible(&callee),
            Err(CrossContractError::InterfaceVersionMismatch)
        );
    }

    #[test]
    fn stake_vault_client_blocks_mismatch_before_action() {
        let client = StakeVaultClient::new(STAKE_VAULT_INTERFACE_VERSION);
        let callee = StakeVaultCallee::new(STAKE_VAULT_INTERFACE_VERSION + 1);

        let result = client.proceed_if_compatible(&callee, || Ok::<_, CrossContractError>(42));

        assert_eq!(result, Err(CrossContractError::InterfaceVersionMismatch));
    }

    #[test]
    fn callers_can_bump_callee_versions_and_have_their_expectations_caught() {
        let client = OracleClient::new(1);
        let callee = OracleCallee::new(2);

        let result = client.proceed_if_compatible(&callee, || Ok::<_, CrossContractError>("ok"));

        assert_eq!(result, Err(CrossContractError::InterfaceVersionMismatch));
    }
}
