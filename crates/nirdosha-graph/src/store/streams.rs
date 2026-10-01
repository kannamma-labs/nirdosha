use super::*;
impl Graph {
    pub fn stream_open(&self, key: &str) -> Result<Value> {
        self.transaction(||{
        let v=self.raw_version()?;
        if key.is_empty()||key.len()>256{return Err(Error::new("SCHEMA_INVALID","Invalid stream client key"));}
        let existing:Option<String>=self.conn.query_row("SELECT id FROM graph_streams WHERE actor=?1 AND epoch=?2 AND client_key=?3",params![self.access.actor,v.epoch,key],|r|r.get(0)).optional()?;
        let stream=if let Some(id)=existing{id}else{
            let (total,mine):(u64,u64)=self.conn.query_row("SELECT COUNT(*),COALESCE(SUM(actor=?1),0) FROM graph_streams WHERE state='open' AND epoch=?2",params![self.access.actor,v.epoch],|r|Ok((r.get(0)?,r.get(1)?)))?;
            if total>=16||mine>=4{return Err(Error::new("LIMIT_EXCEEDED","Open stream quota exceeded"));}
            let id=id("stream")?;self.conn.execute("INSERT INTO graph_streams VALUES(?1,?2,?3,?4,1,'open')",params![id,self.access.actor,v.epoch,key])?;id
        };self.stream_status(&stream)
    })
    }
    pub fn stream_status(&self, key: &str) -> Result<Value> {
        self.check_access(false)?;
        let row: Option<(String, String, u64, String)> = self
            .conn
            .query_row(
                "SELECT actor,epoch,next_sequence,state FROM graph_streams WHERE id=?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let Some((actor, epoch, next, state)) = row else {
            return Err(Error::new("NOT_FOUND", "Stream not found"));
        };
        if actor != self.access.actor || epoch != self.raw_version()?.epoch {
            return Err(Error::new("UNAUTHORIZED", "Stream actor or epoch differs"));
        }
        Ok(json!({"stream_id":key,"next_sequence":next,"state":state,"graph":self.raw_version()?}))
    }
    pub fn stream_append(&self, key: &str, sequence: u64, patch: &Value) -> Result<Value> {
        self.transaction(||{
        let status=self.stream_status(key)?;let hash=hash::structured("patch",patch)?;
        let old:Option<(String,String)>=self.conn.query_row("SELECT hash,receipt FROM graph_stream_receipts WHERE stream_id=?1 AND sequence=?2",params![key,sequence],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((h,r))=old {if h!=hash{return Err(Error::new("SEQUENCE_CONFLICT","Sequence already committed with different content"));}return Ok(serde_json::from_str(&r)?);}
        if status["state"]!="open"{return Err(Error::new("STREAM_CLOSED","Stream is closed"));}
        if status["next_sequence"]!=sequence{return Err(Error::new("OUT_OF_ORDER","Expected contiguous sequence").details(json!({"next_sequence":status["next_sequence"]})));}
        if sequence>=hash::MAX_INTEGER{return Err(Error::new("REVISION_EXHAUSTED","Stream sequence exhausted"));}
        let mut result=self.apply_inner(patch,&format!("{key}:{sequence}"))?;
        result["stream_id"]=key.into();result["sequence"]=sequence.into();result["next_sequence"]=(sequence+1).into();
        self.conn.execute("INSERT INTO graph_stream_receipts VALUES(?1,?2,?3,?4)",params![key,sequence,hash,serde_json::to_string(&result)?])?;
        self.conn.execute("UPDATE graph_streams SET next_sequence=?2 WHERE id=?1",params![key,sequence+1])?;Ok(result)
    })
    }
    pub fn stream_close(&self, key: &str, final_sequence: u64, cancel: bool) -> Result<Value> {
        self.transaction(|| {
            let status = self.stream_status(key)?;
            if status["next_sequence"].as_u64() != final_sequence.checked_add(1) {
                return Err(Error::new(
                    "OUT_OF_ORDER",
                    "Final sequence does not match committed stream",
                ));
            }
            if status["state"] == "open" {
                self.conn.execute(
                    "UPDATE graph_streams SET state=?2 WHERE id=?1",
                    params![key, if cancel { "cancelled" } else { "completed" }],
                )?;
            }
            self.stream_status(key)
        })
    }
}
