# Privacy Policy and Terms of Service

## Privacy policy

hotpath-cloud collects a limited amount of user data. It is used to provide the service and to understand how the service is used, so we can improve it.

Individual users' personally identifiable data stored on our servers consists of their GitHub identifier, login, email address and avatar, together with the GitHub authorization tokens used to check which repositories they can access.

Reports of a repository marked as public on the dashboard are readable by anyone with the link, without logging in. Reports of any other private repository are readable only by GitHub users who can read the repository, and reports of any other public repository only by GitHub users with push access to it.

Network access logs with IP addresses of the website visitors are stored for maintenance purposes. They are permanently removed after 30 days.

Web connections to the application servers are encrypted via SSL, which means that all data in transit is encrypted. The backend database uses encrypted storage.

The website uses cookies. They are used only to keep you logged in and to remember your display preferences.

We use Ahrefs Web Analytics to collect anonymized website visits data on the documentation and blog pages. It does not use cookies and does not store any personally identifiable information.

We use PostHog (EU-hosted) to understand how registered users interact with the hotpath-cloud dashboard and improve the service. We collect dashboard pageviews, public repository names, and pseudonymized user, account and repository identifiers. We do not send user email addresses or GitHub login names as separate identity properties. Tracking is server-side and does not use additional cookies. We process this data based on our legitimate interest in improving the service. You can object at any time by contacting support@hotpath.rs.

The blog offers an optional email newsletter. If you subscribe, your email address and marketing consent are stored by Mailchimp (Intuit Inc., United States), which we use to send the newsletter. The newsletter is shared with pawelurbanek.com. The signup form loads its scripts from Mailchimp's servers only after you open it. We process this data based on your consent, which you can withdraw at any time with the unsubscribe link in any newsletter email or by contacting us. Your data is kept until you unsubscribe. Mailchimp handles it under its own [privacy policy](https://www.intuit.com/privacy/statement/).

Payments for paid plans are processed by Paddle.com, our reseller and Merchant of Record. Paddle receives the purchaser's email address, billing address and payment details and handles them under its own [privacy policy](https://www.paddle.com/legal/privacy). We store the Paddle customer, subscription and transaction identifiers of each purchase, its plan and status, and the payment notifications Paddle sends us, to apply the plan to the repository and to handle refunds.

The service is hosted on Amazon Web Services in the United States (us-east-1), and Hetzner US datacenter, behind Cloudflare.

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

By installing the hotpath GitHub App or logging in to the hotpath-cloud dashboard, you agree to abide by these Terms of Service and to comply with all applicable laws and regulations. You may end the agreement at any time by removing the hotpath GitHub App and no longer using the service. Removing the app does not cancel a paid subscription: cancel it separately, as described under [Subscriptions](#subscriptions).

You may only use the hotpath-cloud service as permitted by law. We may investigate and/or suspend or terminate our service to you at any time if we find your use of our service violates the agreement and/or any policies.

These terms cover the hosted service at hotpath.rs only. The open-source hotpath crates keep their own license.

### Payments

Our order process is conducted by our online reseller Paddle.com. Paddle.com is the Merchant of Record for all our orders. Paddle provides all customer service inquiries and handles returns.

Plans, prices and limits are listed on the [pricing](pricing.md) page. Prices are in USD and exclude VAT; Paddle adds the sales tax that applies at checkout and issues the invoice.

### Subscriptions

A paid plan is a monthly subscription for one repository, billed in advance. It renews automatically each month until it is cancelled.

You can cancel at any time with the link in your Paddle receipt email, on [paddle.net](https://paddle.net), or by contacting support@hotpath.rs. A cancellation takes effect at the end of the billing period already paid for, and nothing is charged after it.

Uninstalling the hotpath GitHub App or removing a repository from it does not cancel the repository's subscription. Its reports are still deleted 7 days after the removal, as described under [Data retention](#data-retention). To stop billing, cancel the subscription in one of the ways above.

When a subscription ends, the repository returns to the Public plan. Its reports and benchmarks stay available on the dashboard. The previous plan's reports history is kept for 30 days; after that, the oldest reports beyond the Public plan's limit are deleted. A private repository on the Public plan no longer accepts new uploads once its free trial is over.

A private repository's free trial lasts 30 days from its first upload and is offered once per repository. No payment details are needed for it, and nothing is charged when it ends.

### Refunds

If you are not happy with a paid plan, you get a full refund within 14 days of your initial subscription purchase, no questions asked.

To request a refund, contact support@hotpath.rs, or Paddle on [paddle.net](https://paddle.net) or through the link in your receipt email.

This policy does not affect your statutory rights as a consumer.

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

When a repository is removed from the hotpath GitHub App (you remove it from the installation, uninstall the app, or delete the repository on GitHub), its reports, benchmarks and policies are permanently deleted from the database after 7 days. Re-adding the repository within that time restores them; added later, it starts as a new repository. GitHub webhook notifications we receive are deleted after 7 days. Encrypted backups are permanently removed after 14 days.

We keep the records needed to process payments, refunds and subscriptions: the Paddle customer, subscription and transaction identifiers, plan and status, and the payment notifications Paddle sends us. A removed repository with a subscription that has not been cancelled keeps its name and plan, but not its reports, until the subscription ends.

If you want to expedite your data removal process, please contact us at support@hotpath.rs.

### License

The software is provided "as is", without warranty of any kind, express or implied, including but not limited to the warranties of merchantability, fitness for a particular purpose and noninfringement. In no event shall the authors or copyright holders be liable for any claim, damages or other liability, whether in an action of contract, tort or otherwise, arising from, out of or in connection with the software or the use or other dealings in the software.
