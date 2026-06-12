# SQLCOST034 — Scalar subquery in SELECT list

**Severity:** medium

Detects per-row scalar subqueries in the `SELECT` projection of downstream dbt models.

## When it fires

- A subquery appears in the select list, e.g. `(select max(amount) from payments p where p.user_id = u.id)`.
- Staging models are exempt; rule targets downstream model paths.

## Fix

Join or pre-aggregate the lookup once, then select the joined column.

```sql
select u.id, p.max_amount
from users u
left join payment_totals p on u.id = p.user_id
```
