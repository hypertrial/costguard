select
  id,
  json_extract_scalar(payload, '$.session_id') as session_id,
  json_extract_scalar(payload, '$.session_id') as session_id_again,
  regexp_extract(user_agent, 'Chrome/([0-9]+)') as chrome_major,
  regexp_replace(user_agent, ' Mobile', '') as cleaned_user_agent
from raw.events
