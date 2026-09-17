use crate::*;
impl Runtime {
    pub(crate) fn save_intents(&self,i:&Instance,event:&str,intents:&[Intent])->Result<()> {
        let actions=intents.iter().map(|i|i.action_id.clone()).collect::<BTreeSet<_>>();
        if actions.len()!=intents.len(){return Err(Error::new("SCHEMA_INVALID","Duplicate outbox action ID"))}
        let mut completed=BTreeSet::new();
        for intent in intents {
            if intent.depends_on.iter().any(|d|!completed.contains(d)){return Err(Error::new("SCHEMA_INVALID","Outbox dependencies must name preceding actions in this event"))}
            let key=hash::structured("runtime-outbox-id",&json!([i.id,event,intent.action_id]))?;
            self.db.execute("INSERT INTO wf_outbox VALUES(?1,?2,?3,?4,'pending',NULL,NULL,NULL)",params![key,i.id,event,serde_json::to_string(intent)?])?;completed.insert(intent.action_id.clone());
        }Ok(())
    }
    /// Claim is committed before any network request. Expired leases for effects
    /// without provider idempotency become unknown and require reconciliation.
    pub fn claim_outbox(&self,lease_seconds:u64)->Result<Option<Value>> {if lease_seconds==0||lease_seconds>300{return Err(Error::new("SCHEMA_INVALID","Lease must be 1–300 seconds"))}self.tx(||{
        let mut q=self.db.prepare("SELECT id,instance,event_key,intent,state,lease_expires FROM wf_outbox WHERE state IN ('pending','in_flight') ORDER BY rowid")?;
        let rows=q.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,Option<u64>>(5)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        for (key,instance,event,intent,status,expires) in rows {let intent:Intent=serde_json::from_str(&intent)?;
            if status=="in_flight" {
                if expires.is_some_and(|e|e>now()){continue}
                if !intent.provider_idempotent {self.db.execute("UPDATE wf_outbox SET state='outcome_unknown',result='lease expired before acknowledgment' WHERE id=?1",[&key])?;continue}
            }
            let mut ready=true;for dependency in &intent.depends_on{let id=hash::structured("runtime-outbox-id",&json!([instance,event,dependency]))?;let status:String=self.db.query_row("SELECT state FROM wf_outbox WHERE id=?1",[id],|r|r.get(0))?;if status!="succeeded"{ready=false}}
            if !ready{continue}let token=id("delivery")?;
            self.db.execute("UPDATE wf_outbox SET state='in_flight',lease_token=?2,lease_expires=?3 WHERE id=?1",params![key,token,now()+lease_seconds])?;
            self.db.execute("INSERT INTO wf_deliveries(outbox_id,lease_token,state,result,recorded_at) VALUES(?1,?2,'in_flight',NULL,?3)",params![key,token,now()])?;
            return Ok(Some(json!({"outbox_id":key,"idempotency_key":key,"lease_token":token,"intent":intent})))
        }Ok(None)
    })}
    pub fn complete_outbox(&self,key:&str,token:&str,outcome:&str,evidence:&Value)->Result<()> {if !["succeeded","failed","outcome_unknown"].contains(&outcome){return Err(Error::new("SCHEMA_INVALID","Invalid delivery outcome"))}self.tx(||{
        let (state,active,previous):(String,Option<String>,Option<String>)=self.db.query_row("SELECT state,lease_token,result FROM wf_outbox WHERE id=?1",[key],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        let content=evidence.to_string();if active.as_deref()==Some(token)&&state==outcome&&previous.as_deref()==Some(&content){return Ok(())}
        if state!="in_flight"||active.as_deref()!=Some(token){return Err(Error::new("REVISION_CONFLICT","Delivery lease is no longer active"))}
        self.db.execute("UPDATE wf_outbox SET state=?2,result=?3 WHERE id=?1",params![key,outcome,content])?;
        self.db.execute("INSERT INTO wf_deliveries(outbox_id,lease_token,state,result,recorded_at) VALUES(?1,?2,?3,?4,?5)",params![key,token,outcome,content,now()])?;Ok(())
    })}
    pub fn outbox(&self,instance:&str)->Result<Vec<Value>>{let mut q=self.db.prepare("SELECT id,intent,state,result FROM wf_outbox WHERE instance=?1 ORDER BY rowid")?;let rows=q.query_map([instance],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?)))?;rows.map(|r|{let(id,intent,state,result)=r?;Ok(json!({"id":id,"intent":serde_json::from_str::<Value>(&intent)?,"state":state,"result":result}))}).collect()}
    /// Operator action with recorded evidence, never an automatic unknown retry.
    pub fn reconcile_outbox(&self,key:&str,outcome:&str,evidence:&Value)->Result<()> {
        if !["pending","succeeded","failed"].contains(&outcome)||evidence.is_null(){return Err(Error::new("SCHEMA_INVALID","Reconciliation needs an explicit outcome and evidence"))}
        self.tx(||{let changed=self.db.execute("UPDATE wf_outbox SET state=?2,result=?3,lease_token=NULL,lease_expires=NULL WHERE id=?1 AND state IN ('outcome_unknown','failed')",params![key,outcome,evidence.to_string()])?;if changed!=1{return Err(Error::new("REVISION_CONFLICT","Action is not awaiting reconciliation"))}self.db.execute("INSERT INTO wf_deliveries(outbox_id,lease_token,state,result,recorded_at) VALUES(?1,'operator-reconciliation',?2,?3,?4)",params![key,outcome,evidence.to_string(),now()])?;Ok(())})
    }
    /// Explicit operator migration. All prior decisions remain in history, but
    /// a new round means none can authorize the new definition or payload.
    pub fn migrate_instance(&self,key:&str,instance:&str,expected_revision:u64,definition:&str,workflow:&str,target:&str,data:Value,operator_credential:&str,hooks:&dyn Hooks)->Result<Value>{self.tx(||{
        let(subject,actor)=self.authenticate(operator_credential)?;if !actor.roles.contains("nirdosha.runtime.operator"){return Err(Error::new("UNAUTHORIZED","Runtime operator role required"))}
        let digest=hash::structured("runtime-migration",&json!([instance,expected_revision,definition,workflow,target,data,subject]))?;if let Some(r)=self.receipt(key,&digest)?{return Ok(r)}
        let mut i=self.instance(instance)?;if i.revision!=expected_revision{return Err(Error::new("REVISION_CONFLICT","Instance changed"))}
        if self.outbox(instance)?.iter().any(|a|["pending","in_flight","outcome_unknown"].contains(&a["state"].as_str().unwrap_or(""))){return Err(Error::new("MIGRATION_BLOCKED","Drain or reconcile outstanding actions before migration"))}
        let state=self.definition(definition)?;let target_state=state.nodes.get(target).filter(|n|n.kind=="WorkflowState"&&n.spec["workflow_id"]==workflow).ok_or_else(||Error::new("SCHEMA_INVALID","Invalid migration target"))?;
        i.definition_id=definition.into();i.workflow_id=workflow.into();i.state_id=target.into();i.data=data;i.round=next(i.round)?;i.revision=next(i.revision)?;
        let event=json!({"migration":true});check(&target_state.spec["entry_conditions"],&context(&i,&actor,&event))?;let mut intents=vec![];run_hooks(target_state,"on_entry",&mut i,&actor,&event,hooks,&mut intents)?;invariants(&state,&i,&actor,&event)?;i.payload_hash=hash::structured("runtime-review-payload",&i.data)?;
        self.save(&i)?;self.save_intents(&i,key,&intents)?;let result=json!({"instance":i,"migration":true});self.commit_receipt(key,&digest,&result,&subject)?;Ok(result)
    })}
}
