# Device Registration Guide

## Device Credentials (from code)

Your device uses the following credentials for OTAA join:

- **Device EUI**: `AC1F09FFFE1BCE23`
- **Application EUI**: `B130A864C5295356`
- **Application Key**: `B726739B78EC4B9E9234E5D35EA9681B`

## How to Check if Device is Registered

### Method 1: Network Server Web Interface

1. **Access the network server web interface**
   - Usually at `http://<gateway-ip>` or `http://localhost`
   - Check your gateway's web interface URL

2. **Navigate to Devices/End Devices section**
   - Look for "Devices", "End Devices", or "Nodes" menu item
   - Search for Device EUI: `AC1F09FFFE1BCE23`

3. **Verify the device exists and check:**
   - Device EUI matches: `AC1F09FFFE1BCE23`
   - Application EUI matches: `B130A864C5295356`
   - Application Key matches: `B726739B78EC4B9E9234E5D35EA9681B`
   - Activation Mode: **OTAA** (not ABP)
   - Class: **A**
   - Status: Should show as "Never seen" or "Inactive" if not joined yet

### Method 2: Check Gateway/Network Server Logs

Look for device-related logs when join attempts occur:

```bash
# If using syslog
grep -i "AC1F09FFFE1BCE23" /var/log/syslog

# Or check network server logs
# Location depends on your setup (ChirpStack, TTN, etc.)
```

If the device is registered, you should see:
- Join request received
- Join accept sent (if credentials match)
- Or join reject (if credentials don't match)

If the device is NOT registered, you might see:
- No logs about the device
- Or "device not found" errors

### Method 3: Network Server API

If your network server has an API, you can query it:

```bash
# Example for ChirpStack (adjust for your server)
curl -X GET "http://localhost:8080/api/devices/AC1F09FFFE1BCE23" \
  -H "Authorization: Bearer <token>"
```

## How to Register the Device

If the device is NOT registered, add it:

1. **Go to Network Server Web Interface**
   - Navigate to "Devices" or "End Devices"
   - Click "Add Device" or "Create Device"

2. **Enter Device Information:**
   - **Device EUI**: `AC1F09FFFE1BCE23`
   - **Application EUI**: `B130A864C5295356`
   - **Application Key**: `B726739B78EC4B9E9234E5D35EA9681B`
   - **Activation Mode**: **OTAA** (Over-The-Air Activation)
   - **Class**: **A**
   - **Frame Counter Width**: 32 (default)
   - **LoRaWAN MAC Version**: 1.0.2 or 1.0.3 (check your gateway)

3. **Save the device**

4. **Verify Application/Service Profile:**
   - Make sure the device is assigned to an Application
   - Check that the Application EUI matches

## Important Notes

### Credential Format

- **Device EUI**: 8 bytes, hex format: `AC1F09FFFE1BCE23`
- **Application EUI**: 8 bytes, hex format: `B130A864C5295356`
- **Application Key**: 16 bytes, hex format: `B726739B78EC4B9E9234E5D35EA9681B`

**⚠️ WARNING**: Make sure there are NO spaces or dashes in the hex strings when entering in the network server!

### What Happens if Device is NOT Registered

- Gateway might receive the join request (if RF is working)
- Network server will **reject** the join request
- You'll see "No join accept received" on the device
- Gateway logs might show "device not found" or "invalid credentials"

### What Happens if Device IS Registered but Credentials Don't Match

- Gateway receives the join request
- Network server processes it but **rejects** due to wrong key
- Device gets "No join accept received"
- Network server logs show "invalid MIC" or "authentication failed"

### Current Status

Based on your logs:
- Gateway shows: `# RF packets received by concentrator: 0`
- This means the gateway's **radio** isn't receiving packets
- This is a **physical layer issue**, not a registration issue

**However**, you still need to register the device for when the RF issue is fixed!

## Troubleshooting

### Device Not Found in Network Server

1. Check if you're looking in the right application/service
2. Verify the Device EUI is correct (case-insensitive, but format matters)
3. Check if device was deleted or disabled

### Join Request Received but Rejected

1. Verify Application Key matches exactly (no spaces, correct case)
2. Check Application EUI matches
3. Verify Device EUI matches
4. Check if device is enabled/active in network server

### Gateway Receives but Network Server Doesn't Process

1. Check network server is running
2. Verify gateway is connected to network server
3. Check network server logs for errors
4. Verify device is in the correct application/service
