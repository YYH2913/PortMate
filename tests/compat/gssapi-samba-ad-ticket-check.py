#!/usr/bin/python3
import argparse
import json

from impacket.krb5.asn1 import AuthorizationData, EncTicketPart, Ticket
from impacket.krb5.ccache import CCache
from impacket.krb5.crypto import Key, _enctype_table
from impacket.krb5.keytab import Keytab
from impacket.krb5.pac import (
    PACTYPE,
    PAC_INFO_BUFFER,
    PAC_LOGON_INFO,
    PAC_UPN_DNS_INFO,
    UPN_DNS_INFO_FULL,
)
from pyasn1.codec.der import decoder


AES_ENCRYPTION_TYPES = {
    17: "aes128-cts-hmac-sha1-96",
    18: "aes256-cts-hmac-sha1-96",
}
AD_IF_RELEVANT = 1
AD_WIN2K_PAC = 128
UPN_DNS_HAS_SAM_AND_SID = 0x2


def fail(message):
    raise SystemExit(f"Samba AD-compatible ticket verification failed: {message}")


def decode_asn1(data, specification, label):
    decoded, trailing = decoder.decode(data, asn1Spec=specification)
    if trailing:
        fail(f"{label} contains {len(trailing)} trailing bytes")
    return decoded


def credential_ticket_details(credential, label):
    ticket = decode_asn1(credential.ticket["data"], Ticket(), f"{label} ticket")
    ticket_encryption_type = int(ticket["enc-part"]["etype"])
    session_encryption_type = int(credential["key"]["keytype"])
    for kind, encryption_type in (
        ("ticket", ticket_encryption_type),
        ("session", session_encryption_type),
    ):
        if encryption_type not in AES_ENCRYPTION_TYPES:
            fail(f"{label} {kind} uses non-AES encryption type {encryption_type}")
    return {
        "ticket": ticket,
        "ticketEncryptionType": AES_ENCRYPTION_TYPES[ticket_encryption_type],
        "sessionEncryptionType": AES_ENCRYPTION_TYPES[session_encryption_type],
    }


def bounded_utf16(buffer, offset, length, label):
    end = offset + length
    if (
        offset < 0
        or length < 0
        or offset % 2 != 0
        or length % 2 != 0
        or end > len(buffer)
    ):
        fail(f"invalid {label} bounds: offset={offset}, length={length}, size={len(buffer)}")
    try:
        return buffer[offset:end].decode("utf-16-le")
    except UnicodeDecodeError as error:
        fail(f"invalid UTF-16 {label}: {error}")


def extract_pac(encrypted_ticket):
    authorization_data = encrypted_ticket["authorization-data"]
    if not authorization_data.hasValue():
        fail("SSH service ticket has no authorization data")
    pac_values = []
    for outer in authorization_data:
        if int(outer["ad-type"]) != AD_IF_RELEVANT:
            continue
        nested = decode_asn1(
            outer["ad-data"].asOctets(),
            AuthorizationData(),
            "AD-IF-RELEVANT authorization data",
        )
        pac_values.extend(
            entry["ad-data"].asOctets()
            for entry in nested
            if int(entry["ad-type"]) == AD_WIN2K_PAC
        )
    if len(pac_values) != 1:
        fail(f"expected one PAC, found {len(pac_values)}")
    return pac_values[0]


def parse_pac(pac):
    if len(pac) < 8:
        fail("PAC header is truncated")
    header = PACTYPE(pac)
    if header["Version"] != 0:
        fail(f"unsupported PAC version {header['Version']}")
    descriptor_end = 8 + header["cBuffers"] * 16
    if descriptor_end > len(pac):
        fail("PAC buffer descriptor table is truncated")
    buffers = {}
    for index in range(header["cBuffers"]):
        descriptor_offset = 8 + index * 16
        descriptor = PAC_INFO_BUFFER(pac[descriptor_offset : descriptor_offset + 16])
        buffer_type = descriptor["ulType"]
        offset = descriptor["Offset"]
        size = descriptor["cbBufferSize"]
        end = offset + size
        if offset % 8 != 0 or offset < descriptor_end or end > len(pac):
            fail(
                f"invalid PAC buffer {buffer_type} bounds: "
                f"offset={offset}, size={size}, PAC size={len(pac)}"
            )
        if buffer_type in buffers:
            fail(f"duplicate PAC buffer type {buffer_type}")
        buffers[buffer_type] = pac[offset:end]
    for required_type in (PAC_LOGON_INFO, PAC_UPN_DNS_INFO):
        if required_type not in buffers:
            fail(f"PAC buffer type {required_type} is missing")
    return buffers


def parse_upn_dns(buffer):
    if len(buffer) < 20:
        fail("PAC UPN_DNS_INFO buffer is truncated")
    info = UPN_DNS_INFO_FULL(buffer)
    if info["Flags"] & UPN_DNS_HAS_SAM_AND_SID == 0:
        fail("PAC UPN_DNS_INFO omits the SAM name and SID extension")
    return {
        "upn": bounded_utf16(buffer, info["UpnOffset"], info["UpnLength"], "UPN"),
        "dnsDomain": bounded_utf16(
            buffer,
            info["DnsDomainNameOffset"],
            info["DnsDomainNameLength"],
            "DNS domain",
        ),
        "samName": bounded_utf16(
            buffer,
            info["SamNameOffset"],
            info["SamNameLength"],
            "SAM name",
        ),
        "sidBytes": info["SidLength"],
    }


parser = argparse.ArgumentParser()
parser.add_argument("--cache", required=True)
parser.add_argument("--service-keytab", required=True)
parser.add_argument("--service-principal", required=True)
parser.add_argument("--expected-canonical-client", required=True)
parser.add_argument("--expected-upn", required=True)
parser.add_argument("--expected-dns-domain", required=True)
parser.add_argument("--expected-sam-name", required=True)
arguments = parser.parse_args()

cache = CCache.loadFile(arguments.cache)
canonical_client = cache.principal.prettyPrint().decode("utf-8")
if canonical_client != arguments.expected_canonical_client:
    fail(
        f"enterprise UPN canonicalized to {canonical_client!r}, "
        f"expected {arguments.expected_canonical_client!r}"
    )

if "@" not in arguments.expected_canonical_client:
    fail("expected canonical client principal has no realm")
canonical_realm = arguments.expected_canonical_client.split("@", 1)[1]
tgt_principal = f"krbtgt/{canonical_realm}@{canonical_realm}"
tgt_credential = cache.getCredential(tgt_principal, anySPN=False)
if tgt_credential is None:
    fail(f"credential cache is missing {tgt_principal}")
tgt_details = credential_ticket_details(tgt_credential, "TGT")

service_credential = cache.getCredential(arguments.service_principal, anySPN=False)
if service_credential is None:
    fail(f"credential cache is missing {arguments.service_principal}")
service_details = credential_ticket_details(service_credential, "SSH service")
service_ticket = service_details.pop("ticket")
tgt_details.pop("ticket")

ticket_encryption_type = int(service_ticket["enc-part"]["etype"])
key_block = Keytab.loadFile(arguments.service_keytab).getKey(
    arguments.service_principal,
    specificEncType=ticket_encryption_type,
    ignoreRealm=False,
)
if key_block is None:
    fail(
        f"service keytab has no encryption type {ticket_encryption_type} key for "
        f"{arguments.service_principal}"
    )
ticket_key = Key(ticket_encryption_type, key_block["keyvalue"]["data"])
decrypted = _enctype_table[ticket_encryption_type].decrypt(
    ticket_key,
    2,
    service_ticket["enc-part"]["cipher"].asOctets(),
)
encrypted_ticket = decode_asn1(decrypted, EncTicketPart(), "decrypted SSH service ticket")
pac = extract_pac(encrypted_ticket)
pac_buffers = parse_pac(pac)
upn_dns = parse_upn_dns(pac_buffers[PAC_UPN_DNS_INFO])
for field, expected in (
    ("upn", arguments.expected_upn),
    ("dnsDomain", arguments.expected_dns_domain),
    ("samName", arguments.expected_sam_name),
):
    if upn_dns[field] != expected:
        fail(f"PAC {field} is {upn_dns[field]!r}, expected {expected!r}")
if upn_dns["sidBytes"] <= 0:
    fail("PAC UPN_DNS_INFO contains an empty SID")

print(
    json.dumps(
        {
            "canonicalClientPrincipal": canonical_client,
            "tgt": tgt_details,
            "sshService": service_details,
            "pacBytes": len(pac),
            "pacBufferTypes": sorted(pac_buffers),
            "pacUpn": upn_dns["upn"],
            "pacDnsDomain": upn_dns["dnsDomain"],
            "pacSamName": upn_dns["samName"],
            "pacSidBytes": upn_dns["sidBytes"],
        },
        sort_keys=True,
    )
)
