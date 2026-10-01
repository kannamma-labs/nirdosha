async function findUserEmail(db, userId) {
  const result = await db.query('SELECT email FROM account WHERE id = ?', [userId]);
  const rows = Array.isArray(result) ? result[0] : (result.rows || result);
  return rows?.[0]?.email ?? null;
}