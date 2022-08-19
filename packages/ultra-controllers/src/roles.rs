// based on https://github.com/CosmWasm/cw-plus/blob/main/packages/controllers/src/admin.rs
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

use cosmwasm_std::{
    attr, Addr, CustomQuery, Deps, DepsMut, MessageInfo, Response, StdError, StdResult, Storage,
};
use cw_storage_plus::{index_list, IndexedMap, Item, MultiIndex};

#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    ActivePool,
    TroveManager,
    Owner,
    StabilityPool,
}

impl ToString for Role {
    fn to_string(&self) -> String {
        match &self {
            Role::ActivePool => "active_pool",
            Role::TroveManager => "trove_manager",
            Role::Owner => "owner",
            Role::StabilityPool => "stability_pool",
        }
        .into()
    }
}

/// Errors returned from Admin
#[derive(Error, Debug, PartialEq)]
pub enum RolesError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("Caller is not {label}")]
    UnauthorizedForRole { label: String },
}

pub type RoleRecord = Addr;

/// stringified role
pub type RolePK<'a> = &'a str;

#[index_list(RoleRecord)]
pub struct RolesIndexes<'a> {
    // find all roles for one address
    // allow for edge case where one address has multiple roles.
    // e.g. `owner` is also `generator`
    roles_by_addr: MultiIndex<'a, Addr, RoleRecord, RolePK<'a>>,
}

pub struct RoleConsumer<'a>(Item<'a, Addr>);

// state/logic
pub struct RoleProvider<'a>(IndexedMap<'a, RolePK<'a>, RoleRecord, RolesIndexes<'a>>);

// this is the core business logic we expose
impl<'a> RoleProvider<'a> {
    pub fn new(namespace: &'a str, roles_by_addr_idx_namespace: &'a str) -> Self {
        RoleProvider(IndexedMap::new(
            namespace,
            RolesIndexes::<'a> {
                roles_by_addr: MultiIndex::new(
                    |addr| addr.clone(),
                    namespace,
                    roles_by_addr_idx_namespace,
                ),
            },
        ))
    }

    pub fn delete(&self, store: &mut dyn Storage, role: &Role) -> StdResult<()> {
        self.0.remove(store, &role.to_string())
    }

    pub fn set(&self, store: &mut dyn Storage, role: &Role, grantee: Addr) -> StdResult<()> {
        self.0.save(store, &role.to_string(), &grantee)
    }

    pub fn get(&self, store: &dyn Storage, role: &Role) -> StdResult<Option<RoleRecord>> {
        self.0.may_load(store, &role.to_string())
    }

    /// Returns Ok(true) if this user has the role, Ok(false) if not and an Error if
    /// we hit an error with Api or Storage usage
    pub fn has_role(&self, store: &dyn Storage, role: &Role, caller: &Addr) -> StdResult<bool> {
        self.0
            .may_load(store, &role.to_string())?
            .map_or_else(|| Ok(false), |addr| Ok(&addr == caller))
    }

    /// Returns Ok(true) if this user has any of the roles, Ok(false) if not and an Error if
    /// we hit an error with Api or Storage usage
    pub fn has_any_role(
        &self,
        store: &dyn Storage,
        roles: &[Role],
        caller: &Addr,
    ) -> StdResult<bool> {
        for role in roles {
            if self.has_role(store, role, caller)? {
                // if any exists, stop iteration and return true result
                return Ok(true);
            } else {
                continue;
            }
        }
        // if nothing was returned, none exists. return false.
        Ok(false)
    }

    /// Like has_any_role but returns RolesError::UnauthorizedForRole if not authorized.
    /// Helper for a nice one-line auth check.
    pub fn assert_any_role(
        &self,
        store: &dyn Storage,
        roles: &[Role],
        caller: &Addr,
    ) -> Result<(), RolesError> {
        for role in roles {
            if !self.has_role(store, &role, caller)? {
                continue;
            } else {
                return Ok(());
            }
        }
        let label = roles
            .into_iter()
            .map(|r| r.to_string())
            .collect::<Vec<String>>()
            .join(" | ");
        Err(RolesError::UnauthorizedForRole { label })
    }

    /// Like has_role but returns RolesError::UnauthorizedForRole if not authorized.
    /// Helper for a nice one-line auth check.
    pub fn assert_role(
        &self,
        store: &dyn Storage,
        role: &Role,
        caller: &Addr,
    ) -> Result<(), RolesError> {
        if !self.has_role(store, &role, caller)? {
            Err(RolesError::UnauthorizedForRole {
                label: role.to_string(),
            })
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use cosmwasm_std::testing::{mock_dependencies, mock_info};
    use cosmwasm_std::Empty;

    #[test]
    fn set_and_get_owner() {
        let mut deps = mock_dependencies();
        let control = RoleProvider::new("foo", "foo__roles_by_addr");

        // initialize and check
        let owner = Addr::unchecked("owner");
        control
            .set(deps.as_mut().storage, &Role::Owner, owner.clone())
            .unwrap();
        let got = control.get(deps.as_ref().storage, &Role::Owner).unwrap();
        assert_eq!(owner, got.unwrap());

        // clear it and check
        control.delete(deps.as_mut().storage, &Role::Owner).unwrap();
        let got = control.get(deps.as_ref().storage, &Role::Owner).unwrap();
        assert_eq!(None, got);
    }

    #[test]
    fn role_checks() {
        let mut deps = mock_dependencies();

        let control = RoleProvider::new("foo", "foo__idx");
        let owner = Addr::unchecked("big boss");
        let imposter = Addr::unchecked("imposter");

        // ensure checks proper with owner set
        control
            .set(deps.as_mut().storage, &Role::Owner, owner.clone())
            .unwrap();
        assert!(control
            .has_role(deps.as_ref().storage, &Role::Owner, &owner)
            .unwrap());
        assert!(
            !(control
                .has_role(deps.as_ref().storage, &Role::Owner, &imposter)
                .unwrap())
        );
        control
            .assert_role(deps.as_ref().storage, &Role::Owner, &owner)
            .unwrap();
        let err = control
            .assert_role(deps.as_ref().storage, &Role::Owner, &imposter)
            .unwrap_err();
        assert_eq!(
            RolesError::UnauthorizedForRole {
                label: Role::Owner.to_string()
            },
            err
        );

        // same checks for `any` variants
        assert!(control
            .has_any_role(
                deps.as_ref().storage,
                &[Role::ActivePool, Role::Owner, Role::StabilityPool],
                &owner
            )
            .unwrap());
        assert!(
            !(control
                .has_any_role(
                    deps.as_ref().storage,
                    &[Role::ActivePool, Role::Owner, Role::StabilityPool],
                    &imposter
                )
                .unwrap())
        );
        control
            .assert_any_role(
                deps.as_ref().storage,
                &[Role::ActivePool, Role::Owner, Role::StabilityPool],
                &owner,
            )
            .unwrap();
        let err = control
            .assert_any_role(
                deps.as_ref().storage,
                &[Role::Owner, Role::ActivePool, Role::StabilityPool],
                &imposter,
            )
            .unwrap_err();
        assert_eq!(
            RolesError::UnauthorizedForRole {
                label: format!(
                    "{} | {} | {}",
                    Role::Owner.to_string(),
                    Role::ActivePool.to_string(),
                    Role::StabilityPool.to_string()
                )
            },
            err
        );

        // ensure checks proper with owner None
        control.delete(deps.as_mut().storage, &Role::Owner).unwrap();
        assert!(
            !(control
                .has_role(deps.as_ref().storage, &Role::Owner, &owner)
                .unwrap())
        );
        assert!(
            !(control
                .has_role(deps.as_ref().storage, &Role::Owner, &imposter)
                .unwrap())
        );
        let err = control
            .assert_role(deps.as_ref().storage, &Role::Owner, &owner)
            .unwrap_err();
        assert_eq!(
            RolesError::UnauthorizedForRole {
                label: Role::Owner.to_string()
            },
            err
        );
        let err = control
            .assert_role(deps.as_ref().storage, &Role::Owner, &imposter)
            .unwrap_err();
        assert_eq!(
            RolesError::UnauthorizedForRole {
                label: Role::Owner.to_string()
            },
            err
        );
    }
}