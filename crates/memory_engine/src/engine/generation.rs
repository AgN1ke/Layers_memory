use super::erasure_flow::scope_erasure_lock_key;
use super::transactional_support::serializable_digest;
use super::*;
use crate::erasure::{erasure_key, WriteGeneration, WRITE_GENERATION_SCHEMA};
use crate::file_storage::FileStorage;

impl MemoryEngine<FileStorage> {
    pub fn write_generation(
        &self,
        session_id: &str,
        personality_scope_id: Option<&str>,
    ) -> Result<WriteGeneration> {
        self.storage.validate_idempotent_session_id(session_id)?;
        let scope = personality_scope_id.unwrap_or(session_id);
        super::erasure_flow::validate_erasure_target("personality_scope_id", scope)?;
        self.ensure_manifest()?;
        self.with_memory_write(scope, session_id, || {
            self.storage
                .validate_ingest_session(session_id, scope, self.storage.host_id())?;
            self.current_write_generation_unlocked(scope, session_id, self.storage.host_id())
        })
    }
}

impl<S: Storage> MemoryEngine<S> {
    pub(super) fn current_write_generation_unlocked(
        &self,
        scope: &str,
        session: &str,
        host: &str,
    ) -> Result<WriteGeneration> {
        let scope_key = erasure_key(MemoryErasureKind::PurgeScope, scope);
        let session_key = erasure_key(MemoryErasureKind::DeleteSession, session);
        Ok(WriteGeneration {
            schema_version: WRITE_GENERATION_SCHEMA.into(),
            scope_generation: self.storage.read_erasure_generation(&scope_key)?,
            session_generation: self.storage.read_erasure_generation(&session_key)?,
            scope_key: serializable_digest(&(host, &scope_key))?,
            session_key: serializable_digest(&(host, &session_key))?,
        })
    }

    pub(super) fn with_memory_write<T>(
        &self,
        scope: &str,
        session: &str,
        f: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        self.with_scope_memory_write(scope, || {
            self.with_resource_lock(session_lock_key(session), || {
                self.require_erasure_complete(MemoryErasureKind::DeleteSession, session)?;
                f()
            })
        })
    }

    pub(super) fn with_scope_memory_write<T>(
        &self,
        scope: &str,
        f: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        self.with_resource_lock(scope_erasure_lock_key(scope), || {
            self.require_erasure_complete(MemoryErasureKind::PurgeScope, scope)?;
            f()
        })
    }

    pub(super) fn with_memory_write_scopes<T>(
        &self,
        scopes: &[String],
        session: &str,
        f: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        self.with_memory_write_targets(scopes, &[session], f)
    }

    pub(super) fn with_memory_write_targets<T>(
        &self,
        scopes: &[String],
        sessions: &[&str],
        f: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let mut keys = scopes
            .iter()
            .map(|scope| scope_erasure_lock_key(scope))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        keys.extend(
            sessions
                .iter()
                .map(|session| session_lock_key(session))
                .collect::<BTreeSet<_>>(),
        );
        let locks = keys
            .iter()
            .map(|key| self.locks.resource(key))
            .collect::<Result<Vec<_>>>()?;
        let _guards = keys
            .iter()
            .zip(&locks)
            .map(|(key, lock)| lock_resource(lock, key))
            .collect::<Result<Vec<_>>>()?;
        for scope in scopes {
            self.require_erasure_complete(MemoryErasureKind::PurgeScope, scope)?;
        }
        for session in sessions {
            self.require_erasure_complete(MemoryErasureKind::DeleteSession, session)?;
        }
        f()
    }

    pub(super) fn with_session_memory_write<T>(
        &self,
        session: &str,
        f: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let scope = self.with_resource_lock(session_lock_key(session), || {
            Ok(self
                .storage
                .read_session_metadata(session)?
                .effective_personality_scope_id()
                .to_owned())
        })?;
        self.with_memory_write(&scope, session, || {
            if self
                .storage
                .read_session_metadata(session)?
                .effective_personality_scope_id()
                != scope
            {
                return Err(MemoryEngineError::OperationStale);
            }
            f()
        })
    }

    pub(super) fn require_erasure_complete(
        &self,
        kind: MemoryErasureKind,
        target: &str,
    ) -> Result<()> {
        if self
            .storage
            .has_erasure_intent(&erasure_key(kind, target))?
        {
            self.memory_operations_recovered
                .store(false, Ordering::Release);
            return Err(MemoryEngineError::OperationIncomplete);
        }
        Ok(())
    }

    pub(super) fn load_task_for_response(&self, task_id: &str) -> Result<PendingTask> {
        let task = self
            .storage
            .load_task(task_id)
            .map_err(|error| self.task_after_erasure_error(task_id, error))?;
        if super::retention::task_payload_compacted(&task)
            || (task.state == TaskState::Completed
                && task.schema_version != PENDING_TASK_SCHEMA_VERSION)
        {
            return Err(MemoryEngineError::OperationResultUnavailable);
        }
        Ok(task)
    }

    pub(super) fn load_sleep_run_for_response(&self, task_id: &str) -> Result<SleepRun> {
        self.storage
            .load_sleep_run(task_id)
            .map_err(|error| self.task_after_erasure_error(task_id, error))
    }

    pub(super) fn task_after_erasure_error(
        &self,
        task_id: &str,
        error: MemoryEngineError,
    ) -> MemoryEngineError {
        match self.storage.was_erased_task(task_id) {
            Ok(true) => MemoryEngineError::OperationStale,
            Ok(false) => error,
            Err(read_error) => read_error,
        }
    }
}
