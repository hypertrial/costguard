select
  coalesce(tr.address, et.address) as address
from {{ ref('stg_transfers') }} tr
full outer join {{ ref('stg_executed_txs') }} et on tr.address = et.address
left join {{ ref('stg_transfers') }} ffb
  on coalesce(tr.address, et.address) = ffb.address
