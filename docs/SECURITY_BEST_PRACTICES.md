# Security Best Practices

## Introduction

Security is paramount when developing and integrating with blockchain applications. This guide provides comprehensive security best practices for developers working with the StellarSwipe protocol.

---

## Table of Contents

1. [Key Management](#key-management)
2. [Smart Contract Security](#smart-contract-security)
3. [Frontend Security](#frontend-security)
4. [API Security](#api-security)
5. [Data Protection](#data-protection)
6. [Operational Security](#operational-security)

---

## Key Management

### Private Key Security

**❌ NEVER DO THIS**:
```typescript
// DON'T hardcode private keys
const SECRET_KEY = "SXXX...";

// DON'T commit keys to version control
const config = {
    privateKey: "SXXX..."
};

// DON'T log private keys
console.log("Key:", privateKey);
```

**✅ DO THIS**:
```typescript
// Use environment variables
const SECRET_KEY = process.env.STELLAR_SECRET_KEY;

// Validate key exists
if (!SECRET_KEY) {
    throw new Error('STELLAR_SECRET_KEY not configured');
}

// Use key management services
import { SecretsManager } from 'aws-sdk';
const secretsManager = new SecretsManager();
const secret = await secretsManager.getSecretValue({
    SecretId: 'stellar-keys'
}).promise();
```

### Key Storage

**Best Practices**:

1. **Use Hardware Wallets** for production keys
2. **Encrypt Keys at Rest**:
```typescript
import crypto from 'crypto';

function encryptKey(key: string, password: string): string {
    const cipher = crypto.createCipher('aes-256-cbc', password);
    let encrypted = cipher.update(key, 'utf8', 'hex');
    encrypted += cipher.final('hex');
    return encrypted;
}

function decryptKey(encrypted: string, password: string): string {
    const decipher = crypto.createDecipher('aes-256-cbc', password);
    let decrypted = decipher.update(encrypted, 'hex', 'utf8');
    decrypted += decipher.final('utf8');
    return decrypted;
}
```

3. **Use Key Derivation**:
```typescript
import { Keypair } from '@stellar/stellar-sdk';
import { pbkdf2Sync } from 'crypto';

function deriveKeypair(password: string, salt: string): Keypair {
    const seed = pbkdf2Sync(password, salt, 100000, 32, 'sha256');
    return Keypair.fromRawEd25519Seed(seed);
}
```

### Multi-Signature Accounts

**Implementation**:
```typescript
import { 
    Account, 
    TransactionBuilder, 
    Operation,
    Networks
} from '@stellar/stellar-sdk';

async function setupMultisig(
    masterKey: Keypair,
    signers: string[],
    threshold: number
) {
    const account = await server.loadAccount(masterKey.publicKey());
    
    const transaction = new TransactionBuilder(account, {
        fee: BASE_FEE,
        networkPassphrase: Networks.PUBLIC
    });
    
    // Add signers
    for (const signer of signers) {
        transaction.addOperation(
            Operation.setOptions({
                signer: {
                    ed25519PublicKey: signer,
                    weight: 1
                }
            })
        );
    }
    
    // Set thresholds
    transaction.addOperation(
        Operation.setOptions({
            masterWeight: 1,
            lowThreshold: threshold,
            medThreshold: threshold,
            highThreshold: threshold
        })
    );
    
    const built = transaction.setTimeout(30).build();
    built.sign(masterKey);
    
    return await server.submitTransaction(built);
}
```

---

## Smart Contract Security

### Input Validation

**Always Validate Inputs**:
```rust
pub fn transfer(env: Env, amount: i128) -> Result<(), Error> {
    // Validate amount
    if amount <= 0 {
        return Err(Error::InvalidAmount);
    }
    
    if amount > MAX_TRANSFER_AMOUNT {
        return Err(Error::AmountTooLarge);
    }
    
    // Proceed with transfer
    Ok(())
}
```

### Access Control

**Implement Proper Authorization**:
```rust
pub fn admin_function(env: Env, caller: Address) -> Result<(), Error> {
    // Require authentication
    caller.require_auth();
    
    // Check authorization
    let admin = get_admin(&env);
    if caller != admin {
        return Err(Error::Unauthorized);
    }
    
    // Proceed with admin action
    Ok(())
}
```

### Reentrancy Protection

**Use Checks-Effects-Interactions Pattern**:
```rust
pub fn withdraw(env: Env, caller: Address) -> Result<(), Error> {
    caller.require_auth();
    
    // CHECKS
    let balance = get_balance(&env, &caller);
    if balance == 0 {
        return Err(Error::InsufficientBalance);
    }
    
    // EFFECTS (update state first)
    set_balance(&env, &caller, 0);
    
    // INTERACTIONS (external calls last)
    transfer_tokens(&env, &caller, balance)?;
    
    Ok(())
}
```

### Integer Overflow Protection

**Use Checked Arithmetic**:
```rust
pub fn add_balance(env: Env, amount: i128) -> Result<(), Error> {
    let current = get_balance(&env);
    
    // Use checked addition
    let new_balance = current.checked_add(amount)
        .ok_or(Error::Overflow)?;
    
    set_balance(&env, new_balance);
    Ok(())
}
```

### Rate Limiting

**Implement Rate Limits**:
```rust
pub fn rate_limited_function(env: Env, caller: Address) -> Result<(), Error> {
    let last_call = get_last_call_time(&env, &caller);
    let current_time = env.ledger().timestamp();
    
    if current_time - last_call < MIN_CALL_INTERVAL {
        return Err(Error::RateLimitExceeded);
    }
    
    set_last_call_time(&env, &caller, current_time);
    
    // Proceed with function
    Ok(())
}
```

---

## Frontend Security

### XSS Prevention

**Sanitize User Input**:
```typescript
import DOMPurify from 'dompurify';

function displayUserContent(content: string) {
    // Sanitize HTML
    const clean = DOMPurify.sanitize(content);
    document.getElementById('content').innerHTML = clean;
}

// Use textContent for plain text
function displayText(text: string) {
    document.getElementById('text').textContent = text;
}
```

### CSRF Protection

**Implement CSRF Tokens**:
```typescript
// Generate CSRF token
function generateCSRFToken(): string {
    return crypto.randomBytes(32).toString('hex');
}

// Validate CSRF token
function validateCSRFToken(token: string): boolean {
    const storedToken = sessionStorage.getItem('csrf_token');
    return token === storedToken;
}

// Include in requests
async function makeSecureRequest(url: string, data: any) {
    const csrfToken = sessionStorage.getItem('csrf_token');
    
    const response = await fetch(url, {
        method: 'POST',
        headers: {
            'Content-Type': 'application/json',
            'X-CSRF-Token': csrfToken
        },
        body: JSON.stringify(data)
    });
    
    return response.json();
}
```

### Content Security Policy

**Set CSP Headers**:
```typescript
// Express.js example
app.use((req, res, next) => {
    res.setHeader(
        'Content-Security-Policy',
        "default-src 'self'; " +
        "script-src 'self' 'unsafe-inline'; " +
        "style-src 'self' 'unsafe-inline'; " +
        "img-src 'self' data: https:; " +
        "connect-src 'self' https://soroban-testnet.stellar.org"
    );
    next();
});
```

### Secure Communication

**Always Use HTTPS**:
```typescript
// Enforce HTTPS
if (window.location.protocol !== 'https:' && 
    window.location.hostname !== 'localhost') {
    window.location.href = 'https:' + window.location.href.substring(window.location.protocol.length);
}

// Use secure WebSocket
const ws = new WebSocket('wss://api.stellarswipe.io/ws');
```

---

## API Security

### Authentication

**Implement JWT Authentication**:
```typescript
import jwt from 'jsonwebtoken';

// Generate token
function generateToken(userId: string): string {
    return jwt.sign(
        { userId },
        process.env.JWT_SECRET,
        { expiresIn: '1h' }
    );
}

// Verify token
function verifyToken(token: string): any {
    try {
        return jwt.verify(token, process.env.JWT_SECRET);
    } catch (error) {
        throw new Error('Invalid token');
    }
}

// Middleware
function authenticateRequest(req, res, next) {
    const token = req.headers.authorization?.split(' ')[1];
    
    if (!token) {
        return res.status(401).json({ error: 'No token provided' });
    }
    
    try {
        const decoded = verifyToken(token);
        req.userId = decoded.userId;
        next();
    } catch (error) {
        return res.status(401).json({ error: 'Invalid token' });
    }
}
```

### Rate Limiting

**Implement API Rate Limiting**:
```typescript
import rateLimit from 'express-rate-limit';

const limiter = rateLimit({
    windowMs: 15 * 60 * 1000, // 15 minutes
    max: 100, // Limit each IP to 100 requests per windowMs
    message: 'Too many requests, please try again later'
});

app.use('/api/', limiter);

// Per-endpoint limits
const strictLimiter = rateLimit({
    windowMs: 60 * 1000, // 1 minute
    max: 10
});

app.post('/api/signals', strictLimiter, createSignal);
```

### Input Validation

**Validate All Inputs**:
```typescript
import { body, validationResult } from 'express-validator';

app.post('/api/signals',
    // Validation rules
    body('assetPair').isString().isLength({ min: 3, max: 20 }),
    body('entryPrice').isFloat({ min: 0 }),
    body('targetPrice').isFloat({ min: 0 }),
    body('stopLoss').isFloat({ min: 0 }),
    
    // Handler
    (req, res) => {
        const errors = validationResult(req);
        if (!errors.isEmpty()) {
            return res.status(400).json({ errors: errors.array() });
        }
        
        // Process request
        createSignal(req.body);
    }
);
```

### SQL Injection Prevention

**Use Parameterized Queries**:
```typescript
// ❌ NEVER DO THIS
const query = `SELECT * FROM signals WHERE id = ${req.params.id}`;

// ✅ DO THIS
const query = 'SELECT * FROM signals WHERE id = ?';
db.query(query, [req.params.id], (err, results) => {
    // Handle results
});

// Or use ORM
const signal = await Signal.findOne({
    where: { id: req.params.id }
});
```

---

## Data Protection

### Encryption at Rest

**Encrypt Sensitive Data**:
```typescript
import crypto from 'crypto';

class DataEncryption {
    private algorithm = 'aes-256-gcm';
    private key: Buffer;
    
    constructor(secret: string) {
        this.key = crypto.scryptSync(secret, 'salt', 32);
    }
    
    encrypt(text: string): string {
        const iv = crypto.randomBytes(16);
        const cipher = crypto.createCipheriv(this.algorithm, this.key, iv);
        
        let encrypted = cipher.update(text, 'utf8', 'hex');
        encrypted += cipher.final('hex');
        
        const authTag = cipher.getAuthTag();
        
        return iv.toString('hex') + ':' + authTag.toString('hex') + ':' + encrypted;
    }
    
    decrypt(encrypted: string): string {
        const parts = encrypted.split(':');
        const iv = Buffer.from(parts[0], 'hex');
        const authTag = Buffer.from(parts[1], 'hex');
        const encryptedText = parts[2];
        
        const decipher = crypto.createDecipheriv(this.algorithm, this.key, iv);
        decipher.setAuthTag(authTag);
        
        let decrypted = decipher.update(encryptedText, 'hex', 'utf8');
        decrypted += decipher.final('utf8');
        
        return decrypted;
    }
}
```

### Secure Data Transmission

**Use TLS/SSL**:
```typescript
import https from 'https';
import fs from 'fs';

const options = {
    key: fs.readFileSync('private-key.pem'),
    cert: fs.readFileSync('certificate.pem'),
    // Enforce strong ciphers
    ciphers: 'ECDHE-RSA-AES128-GCM-SHA256:ECDHE-RSA-AES256-GCM-SHA384',
    honorCipherOrder: true,
    minVersion: 'TLSv1.2'
};

https.createServer(options, app).listen(443);
```

### Data Minimization

**Only Store Necessary Data**:
```typescript
// ❌ DON'T store unnecessary data
interface User {
    id: string;
    email: string;
    password: string; // Store hash, not password
    ssn: string; // Don't store if not needed
    creditCard: string; // Never store
}

// ✅ DO minimize data storage
interface User {
    id: string;
    email: string;
    passwordHash: string;
    publicKey: string;
}
```

---

## Operational Security

### Monitoring and Alerting

**Implement Security Monitoring**:
```typescript
class SecurityMonitor {
    private alerts: Alert[] = [];
    
    logSecurityEvent(event: SecurityEvent) {
        console.log('[SECURITY]', event);
        
        if (event.severity === 'HIGH' || event.severity === 'CRITICAL') {
            this.sendAlert(event);
        }
        
        // Store in database
        this.storeEvent(event);
    }
    
    private sendAlert(event: SecurityEvent) {
        // Send to monitoring service
        // Send email/SMS to security team
        // Trigger incident response
    }
    
    detectAnomalies(transactions: Transaction[]) {
        // Detect unusual patterns
        const unusualVolume = this.detectUnusualVolume(transactions);
        const suspiciousAddresses = this.detectSuspiciousAddresses(transactions);
        
        if (unusualVolume || suspiciousAddresses.length > 0) {
            this.logSecurityEvent({
                type: 'ANOMALY_DETECTED',
                severity: 'HIGH',
                details: { unusualVolume, suspiciousAddresses }
            });
        }
    }
}
```

### Incident Response

**Have an Incident Response Plan**:
```typescript
class IncidentResponse {
    async handleSecurityIncident(incident: Incident) {
        // 1. Assess severity
        const severity = this.assessSeverity(incident);
        
        // 2. Contain the incident
        if (severity === 'CRITICAL') {
            await this.pauseContracts();
            await this.notifyTeam();
        }
        
        // 3. Investigate
        const analysis = await this.investigate(incident);
        
        // 4. Remediate
        await this.remediate(analysis);
        
        // 5. Document
        await this.documentIncident(incident, analysis);
        
        // 6. Post-mortem
        await this.schedulePostMortem(incident);
    }
    
    private async pauseContracts() {
        // Emergency pause mechanism
        console.log('EMERGENCY: Pausing contracts');
        // Implementation
    }
}
```

### Backup and Recovery

**Implement Backup Strategy**:
```typescript
class BackupManager {
    async backupCriticalData() {
        // Backup contract state
        const contractState = await this.exportContractState();
        
        // Backup database
        const dbBackup = await this.backupDatabase();
        
        // Backup keys (encrypted)
        const keyBackup = await this.backupKeys();
        
        // Store in multiple locations
        await this.storeBackup({
            contractState,
            dbBackup,
            keyBackup
        }, [
            's3://backup-primary',
            's3://backup-secondary',
            'local-encrypted-storage'
        ]);
    }
    
    async verifyBackups() {
        // Regularly test backup restoration
        const testRestore = await this.restoreFromBackup('test-environment');
        return testRestore.success;
    }
}
```

### Security Audits

**Regular Security Audits**:
```typescript
class SecurityAudit {
    async performAudit() {
        const results = {
            codeReview: await this.auditCode(),
            dependencyCheck: await this.checkDependencies(),
            configReview: await this.reviewConfiguration(),
            accessControl: await this.auditAccessControl(),
            dataProtection: await this.auditDataProtection()
        };
        
        return this.generateAuditReport(results);
    }
    
    private async checkDependencies() {
        // Check for known vulnerabilities
        // npm audit, snyk, etc.
        return {
            vulnerabilities: [],
            outdated: [],
            recommendations: []
        };
    }
}
```

---

## Security Checklist

### Development Phase

- [ ] Use secure coding practices
- [ ] Implement input validation
- [ ] Add access controls
- [ ] Use checked arithmetic
- [ ] Implement rate limiting
- [ ] Add comprehensive tests
- [ ] Code review by security expert

### Deployment Phase

- [ ] Audit smart contracts
- [ ] Test on testnet thoroughly
- [ ] Set up monitoring
- [ ] Configure alerts
- [ ] Prepare incident response plan
- [ ] Document security procedures
- [ ] Train team on security

### Operational Phase

- [ ] Monitor for anomalies
- [ ] Regular security audits
- [ ] Keep dependencies updated
- [ ] Backup critical data
- [ ] Test disaster recovery
- [ ] Review access logs
- [ ] Update security policies

---

## Resources

### Tools

- **Static Analysis**: Clippy, Cargo Audit
- **Dependency Scanning**: Snyk, Dependabot
- **Monitoring**: Datadog, New Relic
- **Secret Management**: AWS Secrets Manager, HashiCorp Vault

### References

- [OWASP Top 10](https://owasp.org/www-project-top-ten/)
- [Stellar Security Guide](https://developers.stellar.org/docs/security)
- [Soroban Security Best Practices](https://soroban.stellar.org/docs/security)

---

**Document Version**: 1.0.0  
**Last Updated**: 2026-06-01  
**Maintained By**: StellarSwipe Security Team
