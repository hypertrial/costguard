select
  json_extract_scalar(payload, '$.session_id') as session_id,
  json_extract_scalar(payload, '$.session_id') as session_id_dup
from {{ source('raw', 'events') }}
