use crate::ast::{ResourceAccess, SystemDef, SystemParameter, Type};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;

/// Represents the type of resource conflict between two systems
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictType {
    None,
    ReadWrite,  // One system reads, another writes
    WriteRead,  // One system writes, another reads
    WriteWrite, // Both systems write
}

/// Represents an error in the scheduling system
#[derive(Debug, Clone)]
pub enum SchedulerError {
    ResourceConflict {
        system1: String,
        system2: String,
        resource: String,
        conflict_type: ConflictType,
    },
    DeadlockDetected {
        cycle: Vec<String>,
    },
    InvalidResourceAccess {
        system: String,
        resource: String,
        reason: String,
    },
    DuplicateMutableBorrow {
        system1: String,
        system2: String,
        resource: String,
    },
    SchedulingFailure {
        reason: String,
    },
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SchedulerError::ResourceConflict {
                system1,
                system2,
                resource,
                conflict_type,
            } => {
                write!(
                    f,
                    "Resource conflict between systems '{}' and '{}' on resource '{}': {:?}",
                    system1, system2, resource, conflict_type
                )
            }
            SchedulerError::DeadlockDetected { cycle } => {
                write!(f, "Deadlock detected in systems: {}", cycle.join(" -> "))
            }
            SchedulerError::InvalidResourceAccess {
                system,
                resource,
                reason,
            } => {
                write!(
                    f,
                    "Invalid resource access in system '{}' on resource '{}': {}",
                    system, resource, reason
                )
            }
            SchedulerError::DuplicateMutableBorrow {
                system1,
                system2,
                resource,
            } => {
                write!(
                    f,
                    "Multiple mutable borrows of resource '{}' by systems '{}' and '{}'",
                    resource, system1, system2
                )
            }
            SchedulerError::SchedulingFailure { reason } => {
                write!(f, "Scheduling failure: {}", reason)
            }
        }
    }
}

impl std::error::Error for SchedulerError {}

/// Summary of resource usage for monitoring and debugging
#[derive(Debug, Clone)]
pub struct ResourceUsageSummary {
    pub total_resources: usize,
    pub immutable_borrows: usize,
    pub mutable_borrows: usize,
    pub owned_resources: usize,
}

/// Tracks resource usage by systems
#[derive(Debug, Clone)]
pub struct ResourceTracker {
    /// Maps resource name to the systems that have immutable access to it
    immutable_borrows: HashMap<String, HashSet<String>>,
    /// Maps resource name to the system that has mutable access to it (at most one)
    mutable_borrows: HashMap<String, String>,
    /// Maps resource name to the systems that own it
    owned_resources: HashMap<String, String>,
}

impl ResourceTracker {
    pub fn new() -> Self {
        Self {
            immutable_borrows: HashMap::new(),
            mutable_borrows: HashMap::new(),
            owned_resources: HashMap::new(),
        }
    }

    /// Check if a resource access is valid according to borrow checking rules
    pub fn can_access_resource(
        &self,
        resource: &str,
        system: &str,
        access: &ResourceAccess,
    ) -> Result<(), SchedulerError> {
        match access {
            ResourceAccess::Immutable => {
                // Can't have immutable access if someone has mutable access
                if let Some(borrower) = self.mutable_borrows.get(resource) {
                    if borrower != system {
                        return Err(SchedulerError::ResourceConflict {
                            system1: system.to_string(),
                            system2: borrower.clone(),
                            resource: resource.to_string(),
                            conflict_type: ConflictType::ReadWrite,
                        });
                    }
                }
                // Can't have immutable access if someone owns it
                if let Some(owner) = self.owned_resources.get(resource) {
                    if owner != system {
                        return Err(SchedulerError::InvalidResourceAccess {
                            system: system.to_string(),
                            resource: resource.to_string(),
                            reason: format!("Resource is owned by system '{}'", owner),
                        });
                    }
                }
            }
            ResourceAccess::Mutable => {
                // Can't have mutable access if anyone else has any access
                if let Some(immutable_borrowers) = self.immutable_borrows.get(resource) {
                    if !immutable_borrowers.is_empty()
                        && (immutable_borrowers.len() > 1 || !immutable_borrowers.contains(system))
                    {
                        let other_system = immutable_borrowers
                            .iter()
                            .find(|&s| s != system)
                            .unwrap_or(&"unknown".to_string())
                            .clone();
                        return Err(SchedulerError::ResourceConflict {
                            system1: system.to_string(),
                            system2: other_system,
                            resource: resource.to_string(),
                            conflict_type: ConflictType::WriteRead,
                        });
                    }
                }
                if let Some(borrower) = self.mutable_borrows.get(resource) {
                    if borrower != system {
                        return Err(SchedulerError::DuplicateMutableBorrow {
                            system1: system.to_string(),
                            system2: borrower.clone(),
                            resource: resource.to_string(),
                        });
                    }
                }
                if let Some(owner) = self.owned_resources.get(resource) {
                    if owner != system {
                        return Err(SchedulerError::InvalidResourceAccess {
                            system: system.to_string(),
                            resource: resource.to_string(),
                            reason: format!("Resource is owned by system '{}'", owner),
                        });
                    }
                }
            }
            ResourceAccess::Owned => {
                // Can't own a resource if anyone else has any access to it
                if let Some(immutable_borrowers) = self.immutable_borrows.get(resource) {
                    if !immutable_borrowers.is_empty() {
                        let other_system = immutable_borrowers.iter().next().unwrap().clone();
                        return Err(SchedulerError::ResourceConflict {
                            system1: system.to_string(),
                            system2: other_system,
                            resource: resource.to_string(),
                            conflict_type: ConflictType::WriteRead,
                        });
                    }
                }
                if let Some(borrower) = self.mutable_borrows.get(resource) {
                    return Err(SchedulerError::ResourceConflict {
                        system1: system.to_string(),
                        system2: borrower.clone(),
                        resource: resource.to_string(),
                        conflict_type: ConflictType::WriteWrite,
                    });
                }
                if let Some(owner) = self.owned_resources.get(resource) {
                    if owner != system {
                        return Err(SchedulerError::InvalidResourceAccess {
                            system: system.to_string(),
                            resource: resource.to_string(),
                            reason: format!("Resource is already owned by system '{}'", owner),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Add a resource access to the tracker
    pub fn add_access(
        &mut self,
        resource: &str,
        system: &str,
        access: &ResourceAccess,
    ) -> Result<(), SchedulerError> {
        self.can_access_resource(resource, system, access)?;

        match access {
            ResourceAccess::Immutable => {
                self.immutable_borrows
                    .entry(resource.to_string())
                    .or_insert_with(HashSet::new)
                    .insert(system.to_string());
            }
            ResourceAccess::Mutable => {
                self.mutable_borrows
                    .insert(resource.to_string(), system.to_string());
            }
            ResourceAccess::Owned => {
                self.owned_resources
                    .insert(resource.to_string(), system.to_string());
            }
        }
        Ok(())
    }

    /// Remove a resource access from the tracker
    pub fn remove_access(&mut self, resource: &str, system: &str, access: &ResourceAccess) {
        match access {
            ResourceAccess::Immutable => {
                if let Some(borrowers) = self.immutable_borrows.get_mut(resource) {
                    borrowers.remove(system);
                    if borrowers.is_empty() {
                        self.immutable_borrows.remove(resource);
                    }
                }
            }
            ResourceAccess::Mutable => {
                if self.mutable_borrows.get(resource) == Some(&system.to_string()) {
                    self.mutable_borrows.remove(resource);
                }
            }
            ResourceAccess::Owned => {
                if self.owned_resources.get(resource) == Some(&system.to_string()) {
                    self.owned_resources.remove(resource);
                }
            }
        }
    }

    /// Get all resources currently in use
    pub fn get_active_resources(&self) -> HashSet<String> {
        let mut resources = HashSet::new();
        resources.extend(self.immutable_borrows.keys().cloned());
        resources.extend(self.mutable_borrows.keys().cloned());
        resources.extend(self.owned_resources.keys().cloned());
        resources
    }

    /// Get systems that have immutable access to a resource (safe accessor)
    pub fn get_immutable_borrowers(&self, resource: &str) -> Option<&HashSet<String>> {
        self.immutable_borrows.get(resource)
    }

    /// Get the system that has mutable access to a resource (safe accessor)
    pub fn get_mutable_borrower(&self, resource: &str) -> Option<&String> {
        self.mutable_borrows.get(resource)
    }

    /// Get the system that owns a resource (safe accessor)
    pub fn get_resource_owner(&self, resource: &str) -> Option<&String> {
        self.owned_resources.get(resource)
    }

    /// Check if a resource has any active borrows or ownership
    pub fn is_resource_available(&self, resource: &str) -> bool {
        !self.immutable_borrows.contains_key(resource)
            && !self.mutable_borrows.contains_key(resource)
            && !self.owned_resources.contains_key(resource)
    }

    /// Get summary of resource usage for monitoring
    pub fn get_resource_summary(&self) -> ResourceUsageSummary {
        ResourceUsageSummary {
            total_resources: self.get_active_resources().len(),
            immutable_borrows: self.immutable_borrows.len(),
            mutable_borrows: self.mutable_borrows.len(),
            owned_resources: self.owned_resources.len(),
        }
    }
}

/// Represents the current state of the scheduler
#[derive(Debug, Clone)]
pub struct SchedulerState {
    /// Resource tracker for managing borrows and ownership
    resource_tracker: ResourceTracker,
    /// Currently executing systems
    pub executing_systems: HashSet<String>,
    /// Systems waiting to be scheduled
    pub pending_systems: VecDeque<String>,
    /// Completed systems
    pub completed_systems: HashSet<String>,
}

impl SchedulerState {
    pub fn new() -> Self {
        Self {
            resource_tracker: ResourceTracker::new(),
            executing_systems: HashSet::new(),
            pending_systems: VecDeque::new(),
            completed_systems: HashSet::new(),
        }
    }

    /// Add a system to the pending queue
    pub fn enqueue_system(&mut self, system_name: String) {
        if !self.completed_systems.contains(&system_name)
            && !self.executing_systems.contains(&system_name)
        {
            self.pending_systems.push_back(system_name);
        }
    }

    /// Mark a system as completed
    pub fn complete_system(&mut self, system_name: &str) {
        self.executing_systems.remove(system_name);
        self.completed_systems.insert(system_name.to_string());
    }

    /// Check if all systems are completed
    pub fn is_complete(&self) -> bool {
        self.executing_systems.is_empty() && self.pending_systems.is_empty()
    }

    /// Get a reference to the resource tracker (safe accessor)
    pub fn resource_tracker(&self) -> &ResourceTracker {
        &self.resource_tracker
    }

    /// Get a mutable reference to the resource tracker (safe accessor)
    pub fn resource_tracker_mut(&mut self) -> &mut ResourceTracker {
        &mut self.resource_tracker
    }

    /// Try to acquire a resource for a system, enforcing borrow safety
    pub fn try_acquire_resource(
        &mut self,
        resource: &str,
        system: &str,
        access: &ResourceAccess,
    ) -> Result<(), SchedulerError> {
        self.resource_tracker.add_access(resource, system, access)
    }

    /// Release a resource from a system
    pub fn release_resource(&mut self, resource: &str, system: &str, access: &ResourceAccess) {
        self.resource_tracker
            .remove_access(resource, system, access)
    }

    /// Check if a system can access a resource
    pub fn can_system_access_resource(
        &self,
        resource: &str,
        system: &str,
        access: &ResourceAccess,
    ) -> Result<(), SchedulerError> {
        self.resource_tracker
            .can_access_resource(resource, system, access)
    }
}

/// Graph representing conflicts between systems
#[derive(Debug, Clone)]
pub struct ConflictGraph {
    /// Adjacency list: system -> (conflicting_system, conflict_type)
    pub edges: HashMap<String, Vec<(String, ConflictType)>>,
    /// All systems in the graph
    pub systems: HashSet<String>,
}

impl ConflictGraph {
    pub fn new() -> Self {
        Self {
            edges: HashMap::new(),
            systems: HashSet::new(),
        }
    }

    /// Add a system to the graph
    pub fn add_system(&mut self, system: String) {
        self.systems.insert(system.clone());
        self.edges.entry(system).or_insert_with(Vec::new);
    }

    /// Add a conflict edge between two systems
    pub fn add_conflict(&mut self, system1: &str, system2: &str, conflict_type: ConflictType) {
        if conflict_type != ConflictType::None {
            self.edges
                .entry(system1.to_string())
                .or_insert_with(Vec::new)
                .push((system2.to_string(), conflict_type.clone()));

            self.edges
                .entry(system2.to_string())
                .or_insert_with(Vec::new)
                .push((system1.to_string(), conflict_type));
        }
    }

    /// Get systems that conflict with the given system
    pub fn get_conflicts(&self, system: &str) -> Vec<&str> {
        self.edges
            .get(system)
            .map(|conflicts| conflicts.iter().map(|(s, _)| s.as_str()).collect())
            .unwrap_or_default()
    }

    /// Check if there's a conflict between two systems
    pub fn has_conflict(&self, system1: &str, system2: &str) -> bool {
        self.edges
            .get(system1)
            .map(|conflicts| conflicts.iter().any(|(s, _)| s == system2))
            .unwrap_or(false)
    }
}

/// Scheduling priority for systems
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SystemPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

impl Default for SystemPriority {
    fn default() -> Self {
        SystemPriority::Normal
    }
}

/// Scheduling strategy for the scheduler
#[derive(Debug, Clone, PartialEq)]
pub enum SchedulingStrategy {
    /// First-Come-First-Served scheduling
    FCFS,
    /// Priority-based scheduling with preemption
    Priority,
    /// Round-robin scheduling
    RoundRobin,
    /// Shortest Job First (estimate-based)
    SJF,
    /// Work-stealing for load balancing
    WorkStealing,
}

/// Extended system information for scheduling
#[derive(Debug, Clone)]
pub struct SchedulableSystem {
    pub definition: SystemDef,
    pub priority: SystemPriority,
    pub estimated_runtime: Option<u64>, // milliseconds
    pub dependencies: Vec<String>,      // explicit dependencies
    pub last_execution_time: Option<u64>,
}

/// Scheduling statistics for monitoring and optimization
#[derive(Debug, Clone)]
pub struct SchedulingStats {
    pub total_systems: usize,
    pub pending_systems: usize,
    pub executing_systems: usize,
    pub completed_systems: usize,
    pub active_resources: usize,
    pub max_concurrent: usize,
    pub strategy: SchedulingStrategy,
}

/// Main system scheduler with borrow safety analysis
#[derive(Debug)]
pub struct SystemScheduler {
    /// All systems available for scheduling
    systems: HashMap<String, SchedulableSystem>,
    /// Current scheduler state
    state: SchedulerState,
    /// Conflict graph between systems
    conflict_graph: ConflictGraph,
    /// Scheduling strategy
    strategy: SchedulingStrategy,
    /// Maximum number of concurrent systems
    max_concurrent_systems: usize,
    /// System priorities for priority scheduling
    system_priorities: HashMap<String, SystemPriority>,
}

impl SystemScheduler {
    pub fn new() -> Self {
        Self {
            systems: HashMap::new(),
            state: SchedulerState::new(),
            conflict_graph: ConflictGraph::new(),
            strategy: SchedulingStrategy::FCFS,
            max_concurrent_systems: std::thread::available_parallelism()
                .map(|p| p.get())
                .unwrap_or(4),
            system_priorities: HashMap::new(),
        }
    }

    /// Create a new scheduler with custom configuration
    pub fn with_config(strategy: SchedulingStrategy, max_concurrent: usize) -> Self {
        Self {
            systems: HashMap::new(),
            state: SchedulerState::new(),
            conflict_graph: ConflictGraph::new(),
            strategy,
            max_concurrent_systems: max_concurrent,
            system_priorities: HashMap::new(),
        }
    }

    /// Set the scheduling strategy
    pub fn set_strategy(&mut self, strategy: SchedulingStrategy) {
        self.strategy = strategy;
    }

    /// Set system priority
    pub fn set_system_priority(&mut self, system_name: &str, priority: SystemPriority) {
        self.system_priorities
            .insert(system_name.to_string(), priority.clone());

        // Update the stored system if it exists
        if let Some(system) = self.systems.get_mut(system_name) {
            system.priority = priority;
        }
    }

    /// Set explicit dependencies between systems
    pub fn add_dependency(
        &mut self,
        dependent: &str,
        dependency: &str,
    ) -> Result<(), SchedulerError> {
        // Check for circular dependencies
        if self.would_create_cycle(dependent, dependency) {
            return Err(SchedulerError::DeadlockDetected {
                cycle: vec![dependent.to_string(), dependency.to_string()],
            });
        }

        if let Some(system) = self.systems.get_mut(dependent) {
            system.dependencies.push(dependency.to_string());
        }

        Ok(())
    }

    /// Check if adding a dependency would create a cycle
    fn would_create_cycle(&self, from: &str, to: &str) -> bool {
        if from == to {
            return true;
        }

        let mut visited = HashSet::new();
        self.dfs_check_cycle(to, from, &mut visited)
    }

    /// DFS helper for cycle checking
    fn dfs_check_cycle(&self, current: &str, target: &str, visited: &mut HashSet<String>) -> bool {
        if current == target {
            return true;
        }

        if visited.contains(current) {
            return false;
        }

        visited.insert(current.to_string());

        if let Some(system) = self.systems.get(current) {
            for dep in &system.dependencies {
                if self.dfs_check_cycle(dep, target, visited) {
                    return true;
                }
            }
        }

        false
    }

    /// Add a system definition to the scheduler
    pub fn add_system(&mut self, system: SystemDef) -> Result<(), SchedulerError> {
        let system_name = system.name.clone();

        // Validate the system's resource accesses
        self.validate_system_resources(&system)?;

        // Add to conflict graph
        self.conflict_graph.add_system(system_name.clone());

        // Get priority for this system
        let priority = self
            .system_priorities
            .get(&system_name)
            .cloned()
            .unwrap_or_default();

        // Create schedulable system
        let schedulable_system = SchedulableSystem {
            definition: system,
            priority,
            estimated_runtime: None,
            dependencies: Vec::new(),
            last_execution_time: None,
        };

        // Store the system
        self.systems.insert(system_name, schedulable_system);

        Ok(())
    }

    /// Add a system with additional scheduling metadata
    pub fn add_system_with_metadata(
        &mut self,
        system: SystemDef,
        priority: SystemPriority,
        estimated_runtime: Option<u64>,
        dependencies: Vec<String>,
    ) -> Result<(), SchedulerError> {
        let system_name = system.name.clone();

        // Validate the system's resource accesses
        self.validate_system_resources(&system)?;

        // Add to conflict graph
        self.conflict_graph.add_system(system_name.clone());

        // Validate dependencies exist
        for dep in &dependencies {
            if !self.systems.contains_key(dep) {
                return Err(SchedulerError::SchedulingFailure {
                    reason: format!(
                        "Dependency '{}' not found for system '{}'",
                        dep, system_name
                    ),
                });
            }
        }

        // Check for circular dependencies
        for dep in &dependencies {
            if self.would_create_cycle(&system_name, dep) {
                return Err(SchedulerError::DeadlockDetected {
                    cycle: vec![system_name.clone(), dep.clone()],
                });
            }
        }

        // Create schedulable system
        let schedulable_system = SchedulableSystem {
            definition: system,
            priority,
            estimated_runtime,
            dependencies,
            last_execution_time: None,
        };

        // Store the system
        self.systems.insert(system_name, schedulable_system);

        Ok(())
    }

    /// Validate that a system's resource accesses are well-formed
    fn validate_system_resources(&self, system: &SystemDef) -> Result<(), SchedulerError> {
        let mut resource_accesses: HashMap<String, (ResourceAccess, String)> = HashMap::new();

        for param in &system.parameters {
            if let SystemParameter::Resource {
                name,
                resource_type,
                access,
                ..
            } = param
            {
                let type_signature = Self::type_to_resource_id(resource_type);

                // Check for duplicate resource access patterns within the same system
                match resource_accesses.get(name) {
                    Some((existing_access, existing_type)) => {
                        if existing_type != &type_signature {
                            return Err(SchedulerError::InvalidResourceAccess {
                                system: system.name.clone(),
                                resource: name.clone(),
                                reason: format!(
                                    "Resource '{}' declared with conflicting types: '{}' vs '{}'",
                                    name, existing_type, type_signature
                                ),
                            });
                        }

                        if existing_access != access {
                            return Err(SchedulerError::InvalidResourceAccess {
                                system: system.name.clone(),
                                resource: name.clone(),
                                reason: format!(
                                    "System has conflicting access patterns to resource '{}': previously declared as {:?}, now {:?}",
                                    name, existing_access, access
                                ),
                            });
                        }

                        // Duplicate declaration with the same access and type; nothing more to record
                        continue;
                    }
                    None => {
                        resource_accesses.insert(
                            name.clone(),
                            (access.clone(), type_signature),
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Build conflict graph by analyzing resource accesses between all systems
    pub fn build_conflict_graph(&mut self) -> Result<(), SchedulerError> {
        let system_names: Vec<String> = self.systems.keys().cloned().collect();

        for i in 0..system_names.len() {
            for j in (i + 1)..system_names.len() {
                let system1_name = &system_names[i];
                let system2_name = &system_names[j];

                let system1 = &self.systems[system1_name].definition;
                let system2 = &self.systems[system2_name].definition;

                let conflict_type = self.check_system_conflict(system1, system2)?;

                if conflict_type != ConflictType::None {
                    self.conflict_graph
                        .add_conflict(system1_name, system2_name, conflict_type);
                }
            }
        }

        Ok(())
    }

    /// Check for conflicts between two specific systems
    fn check_system_conflict(
        &self,
        system1: &SystemDef,
        system2: &SystemDef,
    ) -> Result<ConflictType, SchedulerError> {
        // Extract resource accesses for both systems
        let resources1 = self.extract_resource_accesses(system1);
        let resources2 = self.extract_resource_accesses(system2);

        // Check for conflicts on shared resources
        for (resource1, access1) in &resources1 {
            if let Some(access2) = resources2.get(resource1) {
                match (access1, access2) {
                    (ResourceAccess::Immutable, ResourceAccess::Immutable) => {
                        // Read-read is safe, continue checking other resources
                        continue;
                    }
                    (ResourceAccess::Immutable, ResourceAccess::Mutable)
                    | (ResourceAccess::Immutable, ResourceAccess::Owned) => {
                        return Ok(ConflictType::ReadWrite);
                    }
                    (ResourceAccess::Mutable, ResourceAccess::Immutable)
                    | (ResourceAccess::Owned, ResourceAccess::Immutable) => {
                        return Ok(ConflictType::WriteRead);
                    }
                    (ResourceAccess::Mutable, ResourceAccess::Mutable)
                    | (ResourceAccess::Mutable, ResourceAccess::Owned)
                    | (ResourceAccess::Owned, ResourceAccess::Mutable)
                    | (ResourceAccess::Owned, ResourceAccess::Owned) => {
                        return Ok(ConflictType::WriteWrite);
                    }
                }
            }
        }

        Ok(ConflictType::None)
    }

    /// Convert a Type to a string for use as a resource identifier
    fn type_to_resource_id(resource_type: &Type) -> String {
        match resource_type {
            Type::Identifier { name, type_args } => {
                if type_args.is_empty() {
                    name.clone()
                } else {
                    format!(
                        "{}[{}]",
                        name,
                        type_args
                            .iter()
                            .map(|t| Self::type_to_resource_id(t))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            }
            Type::Pointer {
                is_mutable,
                pointee,
            } => {
                format!(
                    "{}*{}",
                    if *is_mutable { "mut " } else { "" },
                    Self::type_to_resource_id(pointee)
                )
            }
            Type::RawPointer { pointee } => {
                format!("raw*{}", Self::type_to_resource_id(pointee))
            }
            Type::Optional { inner } => {
                format!("Option[{}]", Self::type_to_resource_id(inner))
            }
            Type::Result { inner } => {
                format!("Result[{}]", Self::type_to_resource_id(inner))
            }
            Type::Tuple(types) => {
                format!(
                    "({})",
                    types
                        .iter()
                        .map(|t| Self::type_to_resource_id(t))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            Type::Matrix {
                element_type,
                dimensions,
            } => {
                format!(
                    "Matrix[{}; {}]",
                    Self::type_to_resource_id(element_type),
                    dimensions
                        .iter()
                        .map(|d| d.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            Type::Function {
                parameters,
                return_type,
            } => {
                format!(
                    "fn({}) -> {}",
                    parameters
                        .iter()
                        .map(|t| Self::type_to_resource_id(t))
                        .collect::<Vec<_>>()
                        .join(", "),
                    Self::type_to_resource_id(return_type)
                )
            }
            Type::Struct { fields } => {
                let field_strs: Vec<String> = fields
                    .iter()
                    .map(|(name, field_type)| {
                        format!("{}: {}", name, Self::type_to_resource_id(field_type))
                    })
                    .collect();
                format!("struct {{{}}}", field_strs.join(", "))
            }
            Type::Enum { variants, .. } => {
                let variant_strs: Vec<String> = variants
                    .iter()
                    .map(|(name, variant_type)| {
                        if let Some(vtype) = variant_type {
                            format!("{}({})", name, Self::type_to_resource_id(vtype))
                        } else {
                            name.clone()
                        }
                    })
                    .collect();
                format!("enum {{{}}}", variant_strs.join(", "))
            }
            Type::Trait {
                associated_types,
                methods,
            } => {
                let assoc_strs: Vec<String> = associated_types.iter().cloned().collect();
                let method_strs: Vec<String> = methods
                    .iter()
                    .map(|(name, method_type)| {
                        format!("{}: {}", name, Self::type_to_resource_id(method_type))
                    })
                    .collect();
                format!(
                    "trait {{types: [{}]; methods: [{}]}}",
                    assoc_strs.join(", "),
                    method_strs.join(", ")
                )
            }
            Type::Reference { is_mutable, inner } => {
                format!(
                    "&{}{}",
                    if *is_mutable { "mut " } else { "" },
                    Self::type_to_resource_id(inner)
                )
            }
            Type::None => "()".to_string(),
        }
    }

    /// Extract resource accesses from a system definition
    /// Uses resource parameter names to align with runtime acquisition
    fn extract_resource_accesses(&self, system: &SystemDef) -> HashMap<String, ResourceAccess> {
        let mut accesses = HashMap::new();

        for param in &system.parameters {
            if let SystemParameter::Resource {
                name,
                access,
                ..
            } = param
            {
                accesses.insert(name.clone(), access.clone());
            }
        }

        accesses
    }

    /// Detect deadlocks in the system dependency graph
    pub fn detect_deadlocks(&self) -> Result<(), SchedulerError> {
        // Use DFS to detect cycles in the conflict graph
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();
        let mut path = Vec::new();

        for system in &self.conflict_graph.systems {
            if !visited.contains(system) {
                if self.dfs_cycle_detection(system, &mut visited, &mut rec_stack, &mut path)? {
                    return Err(SchedulerError::DeadlockDetected { cycle: path });
                }
            }
        }

        Ok(())
    }

    /// DFS helper for cycle detection
    fn dfs_cycle_detection(
        &self,
        system: &str,
        visited: &mut HashSet<String>,
        rec_stack: &mut HashSet<String>,
        path: &mut Vec<String>,
    ) -> Result<bool, SchedulerError> {
        visited.insert(system.to_string());
        rec_stack.insert(system.to_string());
        path.push(system.to_string());

        if let Some(conflicts) = self.conflict_graph.edges.get(system) {
            for (neighbor, _) in conflicts {
                if !visited.contains(neighbor) {
                    if self.dfs_cycle_detection(neighbor, visited, rec_stack, path)? {
                        return Ok(true);
                    }
                } else if rec_stack.contains(neighbor) {
                    // Found a cycle
                    if let Some(cycle_start) = path.iter().position(|s| s == neighbor) {
                        *path = path[cycle_start..].to_vec();
                        path.push(neighbor.clone());
                    }
                    return Ok(true);
                }
            }
        }

        rec_stack.remove(system);
        path.pop();
        Ok(false)
    }

    /// Get the next systems that can be safely scheduled based on current strategy
    pub fn get_schedulable_systems(&self) -> Vec<String> {
        let mut schedulable = Vec::new();

        // Check if we've reached maximum concurrent systems
        if self.state.executing_systems.len() >= self.max_concurrent_systems {
            return schedulable;
        }

        for system_name in &self.state.pending_systems {
            if self.can_schedule_system(system_name) {
                schedulable.push(system_name.clone());
            }
        }

        // Apply scheduling strategy
        match self.strategy {
            SchedulingStrategy::FCFS => {
                // First-come-first-served: already in order
            }
            SchedulingStrategy::Priority => {
                schedulable.sort_by(|a, b| {
                    let priority_a = self
                        .systems
                        .get(a)
                        .map(|s| &s.priority)
                        .unwrap_or(&SystemPriority::Normal);
                    let priority_b = self
                        .systems
                        .get(b)
                        .map(|s| &s.priority)
                        .unwrap_or(&SystemPriority::Normal);
                    priority_b.cmp(priority_a) // Higher priority first
                });
            }
            SchedulingStrategy::SJF => {
                schedulable.sort_by(|a, b| {
                    let runtime_a = self
                        .systems
                        .get(a)
                        .and_then(|s| s.estimated_runtime)
                        .unwrap_or(u64::MAX);
                    let runtime_b = self
                        .systems
                        .get(b)
                        .and_then(|s| s.estimated_runtime)
                        .unwrap_or(u64::MAX);
                    runtime_a.cmp(&runtime_b) // Shorter jobs first
                });
            }
            SchedulingStrategy::RoundRobin => {
                // Round-robin based on last execution time
                schedulable.sort_by(|a, b| {
                    let last_a = self
                        .systems
                        .get(a)
                        .and_then(|s| s.last_execution_time)
                        .unwrap_or(0);
                    let last_b = self
                        .systems
                        .get(b)
                        .and_then(|s| s.last_execution_time)
                        .unwrap_or(0);
                    last_a.cmp(&last_b) // Least recently executed first
                });
            }
            SchedulingStrategy::WorkStealing => {
                // Work-stealing: prioritize systems with many dependencies
                schedulable.sort_by(|a, b| {
                    let deps_a = self
                        .systems
                        .get(a)
                        .map(|s| s.dependencies.len())
                        .unwrap_or(0);
                    let deps_b = self
                        .systems
                        .get(b)
                        .map(|s| s.dependencies.len())
                        .unwrap_or(0);
                    deps_b.cmp(&deps_a) // More dependencies first (to unblock others)
                });
            }
        }

        schedulable
    }

    /// Check if a specific system can be scheduled right now
    fn can_schedule_system(&self, system_name: &str) -> bool {
        // Check if system exists
        let system = match self.systems.get(system_name) {
            Some(s) => s,
            None => return false,
        };

        // Check dependencies are completed
        for dep in &system.dependencies {
            if !self.state.completed_systems.contains(dep) {
                return false;
            }
        }

        // Check if any conflicting systems are currently executing
        for executing_system in &self.state.executing_systems {
            if self
                .conflict_graph
                .has_conflict(system_name, executing_system)
            {
                return false;
            }
        }

        // Check resource availability
        let resource_accesses = self.extract_resource_accesses(&system.definition);
        for (resource, access) in resource_accesses {
            if self
                .state
                .resource_tracker
                .can_access_resource(&resource, system_name, &access)
                .is_err()
            {
                return false;
            }
        }

        true
    }

    /// Schedule a system for execution
    pub fn schedule_system(&mut self, system_name: &str) -> Result<(), SchedulerError> {
        if !self.can_schedule_system(system_name) {
            return Err(SchedulerError::SchedulingFailure {
                reason: format!(
                    "System '{}' cannot be scheduled due to conflicts",
                    system_name
                ),
            });
        }

        // Remove from pending queue
        self.state.pending_systems.retain(|s| s != system_name);

        // Add to executing set
        self.state.executing_systems.insert(system_name.to_string());

        // Update last execution time for round-robin scheduling and get system definition
        let system_definition = if let Some(system) = self.systems.get_mut(system_name) {
            system.last_execution_time = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            );

            // Clone the definition to avoid borrowing issues
            system.definition.clone()
        } else {
            return Err(SchedulerError::SchedulingFailure {
                reason: format!("System '{}' not found", system_name),
            });
        };

        // Extract resource accesses from the cloned definition
        let resource_accesses = self.extract_resource_accesses(&system_definition);

        // Acquire resource accesses
        for (resource, access) in resource_accesses {
            self.state
                .resource_tracker
                .add_access(&resource, system_name, &access)?;
        }

        Ok(())
    }

    /// Complete a system execution and release its resources
    pub fn complete_system(&mut self, system_name: &str) -> Result<(), SchedulerError> {
        if !self.state.executing_systems.contains(system_name) {
            return Err(SchedulerError::SchedulingFailure {
                reason: format!("System '{}' is not currently executing", system_name),
            });
        }

        // Release resource accesses
        if let Some(system) = self.systems.get(system_name) {
            let resource_accesses = self.extract_resource_accesses(&system.definition);
            for (resource, access) in resource_accesses {
                self.state
                    .resource_tracker
                    .remove_access(&resource, system_name, &access);
            }
        }

        // Update state
        self.state.complete_system(system_name);

        Ok(())
    }

    /// Advanced scheduling: schedule multiple systems optimally
    pub fn schedule_batch(&mut self) -> Result<Vec<String>, SchedulerError> {
        let mut scheduled = Vec::new();
        let max_to_schedule = self.max_concurrent_systems - self.state.executing_systems.len();

        if max_to_schedule == 0 {
            return Ok(scheduled);
        }

        let mut schedulable = self.get_schedulable_systems();

        // Apply intelligent batching based on strategy
        match self.strategy {
            SchedulingStrategy::WorkStealing => {
                // Try to schedule systems that unblock the most other systems
                let original_len = schedulable.len();
                for i in 0..std::cmp::min(max_to_schedule, original_len) {
                    if i < schedulable.len() {
                        let system_name = schedulable.remove(i);
                        match self.schedule_system(&system_name) {
                            Ok(_) => {
                                scheduled.push(system_name);
                                // Recalculate schedulable systems after each scheduling
                                schedulable = self.get_schedulable_systems();
                            }
                            Err(_) => {
                                // Skip this system if it can't be scheduled
                                continue;
                            }
                        }
                    }
                }
            }
            _ => {
                // For other strategies, schedule in priority order
                let to_schedule = std::cmp::min(max_to_schedule, schedulable.len());
                for i in 0..to_schedule {
                    let system_name = &schedulable[i];
                    match self.schedule_system(system_name) {
                        Ok(_) => scheduled.push(system_name.clone()),
                        Err(_) => continue,
                    }
                }
            }
        }

        Ok(scheduled)
    }

    /// Get scheduling statistics
    pub fn get_scheduling_stats(&self) -> SchedulingStats {
        SchedulingStats {
            total_systems: self.systems.len(),
            pending_systems: self.state.pending_systems.len(),
            executing_systems: self.state.executing_systems.len(),
            completed_systems: self.state.completed_systems.len(),
            active_resources: self.state.resource_tracker.get_active_resources().len(),
            max_concurrent: self.max_concurrent_systems,
            strategy: self.strategy.clone(),
        }
    }

    /// Estimate completion time for remaining systems
    pub fn estimate_completion_time(&self) -> Option<u64> {
        if self.state.is_complete() {
            return Some(0);
        }

        let mut total_time = 0u64;
        let remaining_systems: Vec<_> = self
            .state
            .pending_systems
            .iter()
            .chain(self.state.executing_systems.iter())
            .collect();

        // Estimate based on system runtimes and dependencies
        for system_name in remaining_systems {
            if let Some(system) = self.systems.get(system_name) {
                total_time += system.estimated_runtime.unwrap_or(1000); // Default 1 second
            }
        }

        // Adjust for parallel execution
        let parallel_factor = self.max_concurrent_systems as u64;
        Some(total_time / parallel_factor.max(1))
    }

    /// Get current scheduler state
    pub fn get_state(&self) -> &SchedulerState {
        &self.state
    }

    /// Get conflict graph
    pub fn get_conflict_graph(&self) -> &ConflictGraph {
        &self.conflict_graph
    }

    /// Enqueue a system for scheduling
    pub fn enqueue_system(&mut self, system_name: String) {
        self.state.enqueue_system(system_name);
    }

    /// Check if scheduling is complete
    pub fn is_complete(&self) -> bool {
        self.state.is_complete()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Type;

    /// Helper function to create a test system with resource parameters
    fn create_test_system(name: &str, resources: Vec<(&str, ResourceAccess)>) -> SystemDef {
        let parameters = resources
            .into_iter()
            .map(|(res_name, access)| SystemParameter::Resource {
                param_type: "resource".to_string(),
                name: res_name.to_string(),
                resource_type: Type::Identifier {
                    name: "TestResource".to_string(),
                    type_args: vec![],
                },
                access,
            })
            .collect();

        SystemDef {
            name: name.to_string(),
            parameters,
            return_type: None,
            body: vec![],
            is_async: false,
        }
    }

    #[test]
    fn test_resource_tracker_immutable_access() {
        let mut tracker = ResourceTracker::new();

        // Multiple immutable accesses should be allowed
        assert!(tracker
            .add_access("resource1", "system1", &ResourceAccess::Immutable)
            .is_ok());
        assert!(tracker
            .add_access("resource1", "system2", &ResourceAccess::Immutable)
            .is_ok());

        // Check that both systems are tracked using safe accessors
        let borrowers = tracker.get_immutable_borrowers("resource1").unwrap();
        assert_eq!(borrowers.len(), 2);
        assert!(borrowers.contains("system1"));
        assert!(borrowers.contains("system2"));
    }

    #[test]
    fn test_resource_tracker_mutable_access_conflict() {
        let mut tracker = ResourceTracker::new();

        // First mutable access should succeed
        assert!(tracker
            .add_access("resource1", "system1", &ResourceAccess::Mutable)
            .is_ok());

        // Second mutable access should fail
        let result = tracker.add_access("resource1", "system2", &ResourceAccess::Mutable);
        assert!(result.is_err());
        match result.unwrap_err() {
            SchedulerError::DuplicateMutableBorrow {
                system1,
                system2,
                resource,
            } => {
                assert_eq!(system1, "system2");
                assert_eq!(system2, "system1");
                assert_eq!(resource, "resource1");
            }
            _ => panic!("Expected DuplicateMutableBorrow error"),
        }
    }

    #[test]
    fn test_resource_tracker_immutable_after_mutable_conflict() {
        let mut tracker = ResourceTracker::new();

        // Mutable access first
        assert!(tracker
            .add_access("resource1", "system1", &ResourceAccess::Mutable)
            .is_ok());

        // Immutable access by different system should fail
        let result = tracker.add_access("resource1", "system2", &ResourceAccess::Immutable);
        assert!(result.is_err());
        match result.unwrap_err() {
            SchedulerError::ResourceConflict {
                system1,
                system2,
                resource,
                conflict_type,
            } => {
                assert_eq!(system1, "system2");
                assert_eq!(system2, "system1");
                assert_eq!(resource, "resource1");
                assert_eq!(conflict_type, ConflictType::ReadWrite);
            }
            _ => panic!("Expected ResourceConflict error"),
        }
    }

    #[test]
    fn test_scheduler_add_system() {
        let mut scheduler = SystemScheduler::new();
        let system = create_test_system(
            "test_system",
            vec![("resource1", ResourceAccess::Immutable)],
        );

        assert!(scheduler.add_system(system).is_ok());
        assert!(scheduler.systems.contains_key("test_system"));
        assert!(scheduler.conflict_graph.systems.contains("test_system"));
    }

    #[test]
    fn test_scheduler_conflict_detection() {
        let mut scheduler = SystemScheduler::new();

        let system1 = create_test_system("system1", vec![("resource1", ResourceAccess::Immutable)]);
        let system2 = create_test_system("system2", vec![("resource1", ResourceAccess::Mutable)]);

        assert!(scheduler.add_system(system1).is_ok());
        assert!(scheduler.add_system(system2).is_ok());
        assert!(scheduler.build_conflict_graph().is_ok());

        // There should be a conflict between system1 and system2
        assert!(scheduler.conflict_graph.has_conflict("system1", "system2"));
    }

    #[test]
    fn test_scheduler_no_conflict_read_read() {
        let mut scheduler = SystemScheduler::new();

        let system1 = create_test_system("system1", vec![("resource1", ResourceAccess::Immutable)]);
        let system2 = create_test_system("system2", vec![("resource1", ResourceAccess::Immutable)]);

        assert!(scheduler.add_system(system1).is_ok());
        assert!(scheduler.add_system(system2).is_ok());
        assert!(scheduler.build_conflict_graph().is_ok());

        // There should be no conflict between two readers
        assert!(!scheduler.conflict_graph.has_conflict("system1", "system2"));
    }

    #[test]
    fn test_scheduler_system_validation() {
        let mut scheduler = SystemScheduler::new();

        // Create a system with conflicting access to the same resource
        let mut system = create_test_system("test_system", vec![]);
        system.parameters.push(SystemParameter::Resource {
            param_type: "resource".to_string(),
            name: "resource1".to_string(),
            resource_type: Type::Identifier {
                name: "TestResource".to_string(),
                type_args: vec![],
            },
            access: ResourceAccess::Immutable,
        });
        system.parameters.push(SystemParameter::Resource {
            param_type: "resource".to_string(),
            name: "resource1".to_string(),
            resource_type: Type::Identifier {
                name: "TestResource".to_string(),
                type_args: vec![],
            },
            access: ResourceAccess::Mutable,
        });

        // This should fail validation
        let result = scheduler.add_system(system);
        assert!(result.is_err());
        match result.unwrap_err() {
            SchedulerError::InvalidResourceAccess {
                system,
                resource,
                reason,
            } => {
                assert_eq!(system, "test_system");
                assert_eq!(resource, "resource1");
                assert!(reason.contains("conflicting access patterns"));
            }
            _ => panic!("Expected InvalidResourceAccess error"),
        }
    }

    #[test]
    fn test_scheduler_conflicting_resource_types() {
        let mut scheduler = SystemScheduler::new();

        let mut system = create_test_system("type_conflict", vec![]);
        system.parameters.push(SystemParameter::Resource {
            param_type: "resource".to_string(),
            name: "resource1".to_string(),
            resource_type: Type::Identifier {
                name: "TestResource".to_string(),
                type_args: vec![],
            },
            access: ResourceAccess::Immutable,
        });
        system.parameters.push(SystemParameter::Resource {
            param_type: "resource".to_string(),
            name: "resource1".to_string(),
            resource_type: Type::Identifier {
                name: "OtherResource".to_string(),
                type_args: vec![],
            },
            access: ResourceAccess::Immutable,
        });

        let result = scheduler.add_system(system);
        assert!(result.is_err());
        match result.unwrap_err() {
            SchedulerError::InvalidResourceAccess {
                system,
                resource,
                reason,
            } => {
                assert_eq!(system, "type_conflict");
                assert_eq!(resource, "resource1");
                assert!(reason.contains("conflicting types"));
            }
            _ => panic!("Expected InvalidResourceAccess error"),
        }
    }
}
