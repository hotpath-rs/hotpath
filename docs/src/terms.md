# Privacy Policy and Terms of Service

## Privacy policy

hotpath-cloud collects a minimum amount of user data needed to work correctly. Data collected is used only to make the service work.

Individual users' personally identifiable data stored on our servers consists of their GitHub identifier, login, email address and avatar, together with the GitHub authorization tokens used to check which repositories they can access.

Reports of a repository marked as public on the dashboard are readable by anyone with the link, without logging in. Reports of any other private repository are readable only by GitHub users who can read the repository, and reports of any other public repository only by GitHub users with push access to it.

Network access logs with IP addresses of the website visitors are stored for maintenance purposes. They are permanently removed after 30 days.

Web connections to the application servers are encrypted via SSL, which means that all data in transit is encrypted. The backend database uses encrypted storage.

The website uses cookies. They are used only to keep you logged in and to remember your display preferences.

We use Ahrefs Web Analytics to collect anonymized website visits data on the documentation and blog pages. It does not use cookies and does not store any personally identifiable information.

The service is hosted on Amazon Web Services in the United States (us-east-1), behind Cloudflare.

We process your account and connection data because it is necessary to provide the service to you, and access logs based on our legitimate interest in operating and securing the service.

You have the right to access, correct, delete or receive a copy of your personal data, and to restrict or object to its processing. You may also lodge a complaint with a competent data protection authority.

For any request related to your data, please contact us at support@hotpath.rs.

The controller of your data is:

PabloWeb Paweł Urbanek\
Okopowa 109/27\
91-849, Łódź, Poland\
VAT EU PL7282665486

### Connected applications

You can connect applications that support the Model Context Protocol (MCP), such as ChatGPT or Claude, to hotpath-cloud. A connection is made only when you approve it on the hotpath.rs consent page, which names the application and what it may do. A connected application can then use the hotpath MCP tools to read the performance reports, diffs and policies of the repositories you can read yourself, under the same access rules as the dashboard. It cannot change anything.

Connected applications receive only credentials issued by hotpath. Your GitHub authorization tokens are never shared with them. Data is sent to an application only in answer to a request it makes, and is then handled under that application's own privacy policy.

For each connection we store the application's identifier and name, the granted permission, when the connection was created and last used, and cryptographic hashes of its authorization codes and access and refresh tokens. While you approve a connection we also briefly store the pending authorization request: the application, the address to return to and its security parameters. These records are linked to your account, so they are personal data even though the credentials are stored only as hashes. We keep them to authorize and manage your connections securely.

You can see and revoke your connections on the dashboard. Revoking a connection takes effect immediately and deletes its tokens, but it does not erase data the application has already received; for that, contact the application's provider.

Authorization requests, codes and tokens are short-lived and deleted automatically once they expire. A connection in regular use stays authorized until you revoke it. One unused for 90 days, with no valid tokens left, is deleted.

## Terms of Service

hotpath-cloud provides hosted performance reports, a GitHub integration and related performance analysis services. Using the service requires internet access, a modern web browser and a GitHub account.

By installing the hotpath GitHub App or logging in to the hotpath-cloud dashboard, you agree to abide by these Terms of Service and to comply with all applicable laws and regulations. You may end the agreement at any time by removing the hotpath GitHub App and no longer using the service.

You may only use the hotpath-cloud service as permitted by law. We may investigate and/or suspend or terminate our service to you at any time if we find your use of our service violates the agreement and/or any policies.

These terms cover the hosted service at hotpath.rs only. The open-source hotpath crates keep their own license.

### Limitations of use

By using the hotpath-cloud service, you warrant on behalf of yourself, your users, and other parties you represent that you will not:

- knowingly or negligently use this website or any of its associated services in a way that abuses or disrupts our networks or any other service hotpath-cloud provides;
- use this website or its associated services to transmit or publish any harassing, indecent, obscene, fraudulent, or unlawful material;
- use this website or its associated services in violation of any applicable laws or regulations;
- use this website in conjunction with sending unauthorized advertising or spam;
- use this website or its associated services in such a way that may infringe the privacy, intellectual property rights, or other rights of third parties.

### Complaints

Complaints regarding the service may be submitted to support@hotpath.rs. Please provide enough information to identify and investigate the issue. We respond within 14 days.

### Changes to terms and services

We reserve the right to review and amend any of these Terms of Service at our sole discretion. Upon doing so, we will update this page. Any changes to these Terms of Service will take effect immediately from the date of publication.

We reserve the right to modify and discontinue offering or updating the service at any time without notice. We will not be liable to you or to any third party for any modification, suspension or discontinuance of the service.

### Data retention

If you remove the hotpath GitHub App from your account or organization, all its data will be automatically removed from the database after 7 days. Encrypted backups are permanently removed after 14 days.

If you want to expedite your data removal process, please contact us at support@hotpath.rs.

### License

The software is provided "as is", without warranty of any kind, express or implied, including but not limited to the warranties of merchantability, fitness for a particular purpose and noninfringement. In no event shall the authors or copyright holders be liable for any claim, damages or other liability, whether in an action of contract, tort or otherwise, arising from, out of or in connection with the software or the use or other dealings in the software.
