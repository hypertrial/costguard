select * from (select token from delta_prod.balancer_v2_arbitrum.events) b
left join delta_prod.tokens.erc20 t on t.contract_address = b.token
