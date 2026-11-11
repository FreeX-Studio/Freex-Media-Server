/*
 * Copyright (c) 2026 MirraNET, Niklas Linz. All rights reserved.
 *
 * This file is part of the MirraNET project and is licensed under the
 * GNU Lesser General Public License v3.0 (LGPLv3).
 *
 * You may use, distribute and modify this code under the terms
 * of the LGPLv3 license. You should have received a copy of the
 * license along with this file. If not, see <https://www.gnu.org/licenses/lgpl-3.0.html>
 * or contact: niklas.linz@mirranet.de
 */

//! The fixed XML of the media server: device description, service descriptions (SCPD), SOAP
//! envelopes and faults, and reading the arguments of a SOAP request.

pub const DEVICE_TYPE: &str = "urn:schemas-upnp-org:device:MediaServer:1";
pub const CD: &str = "urn:schemas-upnp-org:service:ContentDirectory:1";
pub const CM: &str = "urn:schemas-upnp-org:service:ConnectionManager:1";

/// Escapes text for XML content and attributes.
pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            // Control characters are not allowed in XML 1.0.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => o.push(c),
        }
    }
    o
}

fn unesc(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// The text of the first element `name` (any namespace prefix) in a SOAP body.
pub fn arg(body: &str, name: &str) -> Option<String> {
    let mut from = 0;
    while let Some(i) = body[from..].find('<').map(|i| i + from) {
        let rest = &body[i + 1..];
        let end = rest.find('>')?;
        let tag = &rest[..end];
        from = i + 1;
        if tag.starts_with('/') || tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        let tag_name = tag.split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
        let local = tag_name.rsplit(':').next().unwrap_or("");
        if local != name {
            continue;
        }
        if tag.ends_with('/') {
            return Some(String::new());
        }
        let content = &rest[end + 1..];
        let close = content.find(&format!("</{tag_name}>"))?;
        return Some(unesc(content[..close].trim()));
    }
    None
}

/// A SOAP answer with the output arguments (already escaped values).
pub fn soap(service: &str, action: &str, args: &[(&str, String)]) -> String {
    let mut out = String::new();
    for (k, v) in args {
        out.push_str(&format!("<{k}>{v}</{k}>"));
    }
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?><s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/"><s:Body><u:{action}Response xmlns:u="{service}">{out}</u:{action}Response></s:Body></s:Envelope>"#
    )
}

/// A UPnP error (401 invalid action, 402 invalid arguments, 701 no such object, ...).
pub fn fault(code: u16, text: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?><s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/"><s:Body><s:Fault><faultcode>s:Client</faultcode><faultstring>UPnPError</faultstring><detail><UPnPError xmlns="urn:schemas-upnp-org:control-1-0"><errorCode>{code}</errorCode><errorDescription>{}</errorDescription></UPnPError></detail></s:Fault></s:Body></s:Envelope>"#,
        esc(text)
    )
}

/// The device description, `/dlna/description.xml`.
pub fn description(name: &str, udn: &str, version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<root xmlns="urn:schemas-upnp-org:device-1-0" xmlns:dlna="urn:schemas-dlna-org:device-1-0" xmlns:sec="http://www.sec.co.kr/dlna">
  <specVersion><major>1</major><minor>0</minor></specVersion>
  <device>
    <deviceType>{DEVICE_TYPE}</deviceType>
    <dlna:X_DLNADOC>DMS-1.50</dlna:X_DLNADOC>
    <friendlyName>{name}</friendlyName>
    <manufacturer>FreeX</manufacturer>
    <manufacturerURL>https://github.com/FreeX-Studio</manufacturerURL>
    <modelDescription>FreeX Media Server</modelDescription>
    <modelName>FreeX Media Server</modelName>
    <modelNumber>{version}</modelNumber>
    <modelURL>https://github.com/FreeX-Studio/Freex-Media-Server</modelURL>
    <serialNumber>{serial}</serialNumber>
    <UDN>{udn}</UDN>
    <sec:ProductCap>smi,DCM10,getMediaInfo.sec,getCaptionInfo.sec</sec:ProductCap>
    <sec:X_ProductCap>smi,DCM10,getMediaInfo.sec,getCaptionInfo.sec</sec:X_ProductCap>
    <iconList>
      <icon><mimetype>image/png</mimetype><width>192</width><height>192</height><depth>24</depth><url>/dlna/icon.png</url></icon>
    </iconList>
    <serviceList>
      <service>
        <serviceType>{CD}</serviceType>
        <serviceId>urn:upnp-org:serviceId:ContentDirectory</serviceId>
        <SCPDURL>/dlna/cd.xml</SCPDURL>
        <controlURL>/dlna/control/cd</controlURL>
        <eventSubURL>/dlna/event/cd</eventSubURL>
      </service>
      <service>
        <serviceType>{CM}</serviceType>
        <serviceId>urn:upnp-org:serviceId:ConnectionManager</serviceId>
        <SCPDURL>/dlna/cm.xml</SCPDURL>
        <controlURL>/dlna/control/cm</controlURL>
        <eventSubURL>/dlna/event/cm</eventSubURL>
      </service>
    </serviceList>
  </device>
</root>
"#,
        name = esc(name),
        serial = esc(udn.trim_start_matches("uuid:")),
    )
}

fn action(name: &str, args: &[(&str, &str, &str)]) -> String {
    let a: String = args
        .iter()
        .map(|(n, dir, var)| {
            format!("<argument><name>{n}</name><direction>{dir}</direction><relatedStateVariable>{var}</relatedStateVariable></argument>")
        })
        .collect();
    format!("<action><name>{name}</name><argumentList>{a}</argumentList></action>")
}

fn var(name: &str, ty: &str, events: bool, allowed: &[&str]) -> String {
    let list = if allowed.is_empty() {
        String::new()
    } else {
        format!(
            "<allowedValueList>{}</allowedValueList>",
            allowed.iter().map(|v| format!("<allowedValue>{v}</allowedValue>")).collect::<String>()
        )
    };
    format!(
        r#"<stateVariable sendEvents="{}"><name>{name}</name><dataType>{ty}</dataType>{list}</stateVariable>"#,
        if events { "yes" } else { "no" }
    )
}

fn scpd(actions: &[String], vars: &[String]) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?><scpd xmlns="urn:schemas-upnp-org:service-1-0"><specVersion><major>1</major><minor>0</minor></specVersion><actionList>{}</actionList><serviceStateTable>{}</serviceStateTable></scpd>"#,
        actions.concat(),
        vars.concat()
    )
}

/// The ContentDirectory service description; `X_SetBookmark` is the resume position of Samsung TVs.
pub fn content_directory() -> String {
    scpd(
        &[
            action(
                "Browse",
                &[
                    ("ObjectID", "in", "A_ARG_TYPE_ObjectID"),
                    ("BrowseFlag", "in", "A_ARG_TYPE_BrowseFlag"),
                    ("Filter", "in", "A_ARG_TYPE_Filter"),
                    ("StartingIndex", "in", "A_ARG_TYPE_Index"),
                    ("RequestedCount", "in", "A_ARG_TYPE_Count"),
                    ("SortCriteria", "in", "A_ARG_TYPE_SortCriteria"),
                    ("Result", "out", "A_ARG_TYPE_Result"),
                    ("NumberReturned", "out", "A_ARG_TYPE_Count"),
                    ("TotalMatches", "out", "A_ARG_TYPE_Count"),
                    ("UpdateID", "out", "A_ARG_TYPE_UpdateID"),
                ],
            ),
            action("GetSearchCapabilities", &[("SearchCaps", "out", "SearchCapabilities")]),
            action("GetSortCapabilities", &[("SortCaps", "out", "SortCapabilities")]),
            action("GetSystemUpdateID", &[("Id", "out", "SystemUpdateID")]),
            action(
                "X_SetBookmark",
                &[
                    ("CategoryType", "in", "A_ARG_TYPE_CategoryType"),
                    ("RID", "in", "A_ARG_TYPE_RID"),
                    ("ObjectID", "in", "A_ARG_TYPE_ObjectID"),
                    ("PosSecond", "in", "A_ARG_TYPE_PosSec"),
                ],
            ),
        ],
        &[
            var("A_ARG_TYPE_ObjectID", "string", false, &[]),
            var("A_ARG_TYPE_Result", "string", false, &[]),
            var("A_ARG_TYPE_BrowseFlag", "string", false, &["BrowseMetadata", "BrowseDirectChildren"]),
            var("A_ARG_TYPE_Filter", "string", false, &[]),
            var("A_ARG_TYPE_SortCriteria", "string", false, &[]),
            var("A_ARG_TYPE_Index", "ui4", false, &[]),
            var("A_ARG_TYPE_Count", "ui4", false, &[]),
            var("A_ARG_TYPE_UpdateID", "ui4", false, &[]),
            var("A_ARG_TYPE_CategoryType", "ui4", false, &[]),
            var("A_ARG_TYPE_RID", "ui4", false, &[]),
            var("A_ARG_TYPE_PosSec", "ui4", false, &[]),
            var("SearchCapabilities", "string", false, &[]),
            var("SortCapabilities", "string", false, &[]),
            var("SystemUpdateID", "ui4", true, &[]),
        ],
    )
}

pub fn connection_manager() -> String {
    scpd(
        &[
            action("GetProtocolInfo", &[("Source", "out", "SourceProtocolInfo"), ("Sink", "out", "SinkProtocolInfo")]),
            action("GetCurrentConnectionIDs", &[("ConnectionIDs", "out", "CurrentConnectionIDs")]),
            action(
                "GetCurrentConnectionInfo",
                &[
                    ("ConnectionID", "in", "A_ARG_TYPE_ConnectionID"),
                    ("RcsID", "out", "A_ARG_TYPE_RcsID"),
                    ("AVTransportID", "out", "A_ARG_TYPE_AVTransportID"),
                    ("ProtocolInfo", "out", "A_ARG_TYPE_ProtocolInfo"),
                    ("PeerConnectionManager", "out", "A_ARG_TYPE_ConnectionManager"),
                    ("PeerConnectionID", "out", "A_ARG_TYPE_ConnectionID"),
                    ("Direction", "out", "A_ARG_TYPE_Direction"),
                    ("Status", "out", "A_ARG_TYPE_ConnectionStatus"),
                ],
            ),
        ],
        &[
            var("SourceProtocolInfo", "string", true, &[]),
            var("SinkProtocolInfo", "string", true, &[]),
            var("CurrentConnectionIDs", "string", true, &[]),
            var(
                "A_ARG_TYPE_ConnectionStatus",
                "string",
                false,
                &["OK", "ContentFormatMismatch", "InsufficientBandwidth", "UnreliableChannel", "Unknown"],
            ),
            var("A_ARG_TYPE_ConnectionManager", "string", false, &[]),
            var("A_ARG_TYPE_Direction", "string", false, &["Input", "Output"]),
            var("A_ARG_TYPE_ProtocolInfo", "string", false, &[]),
            var("A_ARG_TYPE_ConnectionID", "i4", false, &[]),
            var("A_ARG_TYPE_AVTransportID", "i4", false, &[]),
            var("A_ARG_TYPE_RcsID", "i4", false, &[]),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soap_arguments() {
        let body = r#"<?xml version="1.0"?><s:Envelope xmlns:s="x"><s:Body><u:Browse xmlns:u="urn:schemas-upnp-org:service:ContentDirectory:1"><ObjectID>lib:a&amp;b</ObjectID><BrowseFlag>BrowseDirectChildren</BrowseFlag><Filter>*</Filter><StartingIndex>10</StartingIndex><RequestedCount>0</RequestedCount><SortCriteria/></u:Browse></s:Body></s:Envelope>"#;
        assert_eq!(arg(body, "ObjectID").as_deref(), Some("lib:a&b"));
        assert_eq!(arg(body, "StartingIndex").as_deref(), Some("10"));
        assert_eq!(arg(body, "SortCriteria").as_deref(), Some(""));
        assert_eq!(arg(body, "Missing"), None);
        assert_eq!(arg("<a:ObjectID xmlns:a=\"x\">0</a:ObjectID>", "ObjectID").as_deref(), Some("0"));
    }

    #[test]
    fn escaping() {
        assert_eq!(esc("Tom & Jerry <3 \"x\"\u{1}"), "Tom &amp; Jerry &lt;3 &quot;x&quot;");
    }

    #[test]
    fn descriptions_are_xml() {
        for x in [description("A & B", "uuid:1", "0.21.0"), content_directory(), connection_manager(), fault(701, "No such object")] {
            assert!(x.starts_with("<?xml"), "{x}");
            assert!(!x.contains("A & B"));
        }
    }
}
