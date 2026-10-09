#!/usr/bin/python3
import sys

from ldb import FLAG_MOD_REPLACE, Message, MessageElement
from samba.auth import system_session
from samba.param import LoadParm
from samba.samdb import SamDB


AES_SUPPORTED_ENCRYPTION_TYPES = 0x18
EXPECTED_ACCOUNTS = {
    "LOCALHOST$": None,
    "portmate": "portmate@portmate.test",
}


def fail(message):
    raise SystemExit(f"Samba AD-compatible configuration failed: {message}")


load_parameters = LoadParm()
load_parameters.load_default()
samdb = SamDB(
    url="/var/lib/samba/private/sam.ldb",
    session_info=system_session(),
    lp=load_parameters,
)
results = samdb.search(
    base=samdb.domain_dn(),
    expression="(|(sAMAccountName=portmate)(sAMAccountName=LOCALHOST$))",
    attrs=["sAMAccountName", "userPrincipalName"],
)
accounts = {}
for result in results:
    account_name = str(result["sAMAccountName"][0])
    if account_name in accounts:
        fail(f"duplicate account {account_name}")
    accounts[account_name] = result

if set(accounts) != set(EXPECTED_ACCOUNTS):
    fail(f"unexpected account set {sorted(accounts)}")

for account_name, expected_upn in EXPECTED_ACCOUNTS.items():
    account = accounts[account_name]
    if expected_upn is not None:
        upns = [str(value) for value in account.get("userPrincipalName", [])]
        if upns != [expected_upn]:
            fail(f"unexpected UPN for {account_name}: {upns}")
    message = Message(account.dn)
    message["msDS-SupportedEncryptionTypes"] = MessageElement(
        str(AES_SUPPORTED_ENCRYPTION_TYPES),
        FLAG_MOD_REPLACE,
        "msDS-SupportedEncryptionTypes",
    )
    samdb.modify(message)

sys.stdout.write("Samba AD-compatible AES account policy configured\n")
