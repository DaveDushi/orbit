/**
 * The public Terms of Service and Privacy Policy (no sign-in). The apps link to these paths on the Orbit server they
 * use, so keep them stable. Words in [brackets] are placeholders that the operator fills before the pages go live.
 */
export const LEGAL_PATHS = { terms: '/terms', privacy: '/privacy' } as const

/** Shows the "Draft" banner. Set to false when the operator has approved the text and filled the placeholders. */
export const LEGAL_DRAFT = true

/** A paragraph, or a list of bullet points. */
export type LegalBlock = string | string[]
export type LegalSection = { title: string; body: LegalBlock[] }
export type LegalDocument = { title: string; intro: string; sections: LegalSection[] }

const operator = 'EasyToManage ([Address])'
const contact = 'info@easytomanage.com'

export const TERMS: LegalDocument = {
  title: 'Terms of Service',
  intro: `These terms apply when you use this Orbit server. ${operator} ("we", "us") runs it as a private workspace tool for our team and for the people we invite.`,
  sections: [
    {
      title: '1. The service',
      body: [
        'Orbit is a workspace for tasks, docs, files (Drive), chat and an optional AI assistant. It runs on a server that we operate. It is not a public service: you get an account only from us or from an admin of a workspace on this server.',
      ],
    },
    {
      title: '2. Your account',
      body: [
        [
          'Give a correct name and email address, and keep your password and two-factor codes secret.',
          'You are responsible for what is done with your account, also through API tokens, AI clients and apps that you connect.',
          'Tell us at once if you think that somebody else uses your account.',
          'You must be at least [Minimum age] years old to use Orbit.',
        ],
      ],
    },
    {
      title: '3. Acceptable use',
      body: [
        'Do not use Orbit to:',
        [
          'break the law or the rights of other people, or share content that you have no right to share;',
          'upload malware, or try to get into accounts, workspaces or data that are not yours;',
          'attack, overload or scan the server, or get around its limits and security controls;',
          'send spam or harass other people.',
        ],
      ],
    },
    {
      title: '4. Your content',
      body: [
        'You keep the rights to the content that you put into Orbit (tasks, docs, messages, files). You give us permission to store, copy (for backups), process and show this content as necessary to operate Orbit for you and your workspace, and to send it to the services that you or your workspace admins turn on (see the Privacy Policy).',
        'The owners and admins of a workspace manage its members and content. Content that you share in a workspace stays in the workspace when you leave it.',
      ],
    },
    {
      title: '5. AI features',
      body: [
        'The AI assistant and the AI clients that you connect can make mistakes. Check their answers before you rely on them. When the assistant acts for you (for example, it creates a task or sends a message), it acts with your permissions, and you are responsible for the request.',
      ],
    },
    {
      title: '6. Services of other companies',
      body: [
        'Some features use services of other companies (for example OpenAI for the AI assistant, GitHub for repository sync, Apple and browser vendors for push notifications). Their own terms apply to their services. We are not responsible for them.',
      ],
    },
    {
      title: '7. Availability and changes',
      body: [
        'We provide Orbit "as is" and "as available". We do not promise that it is always available or free of errors. We can change, add or remove features at any time. We make backups, but keep your own copy of content that is important to you.',
      ],
    },
    {
      title: '8. Suspension and end of access',
      body: [
        'Workspace owners and admins can suspend or remove members. We can suspend or end your access if you break these terms, or if we stop operating Orbit. You can ask us to close your account (see the Privacy Policy).',
      ],
    },
    {
      title: '9. Liability',
      body: [
        'To the extent that the law allows it, we are not liable for indirect damage, loss of data or loss of profit that comes from the use of Orbit. Nothing in these terms limits a liability that the law does not let us limit.',
      ],
    },
    {
      title: '10. Law',
      body: ['These terms are governed by the law of [Governing law / country].'],
    },
    {
      title: '11. Changes to these terms',
      body: ['We can change these terms. The date at the top shows the current version. If you use Orbit after a change, the new terms apply.'],
    },
    {
      title: '12. Contact',
      body: [`Questions about these terms: write to ${contact}.`],
    },
  ],
}

export const PRIVACY: LegalDocument = {
  title: 'Privacy Policy',
  intro: `This policy tells what data this Orbit server keeps, why, and who else gets it. ${operator} ("we", "us") operates the server and is responsible for the data on it.`,
  sections: [
    {
      title: '1. Data that Orbit stores',
      body: [
        [
          'Account: your name, email address, profile picture, status and settings. Your password is stored only as a secure hash. If you turn them on: two-factor (authenticator app) settings, recovery codes and passkeys.',
          'Sign-in: your sessions (when they started and when they were last used). Failed sign-ins are recorded with the email address and IP address for a short time, to stop password guessing.',
          'Workspace content: tasks, comments, projects, docs and their earlier versions, chat messages, threads, direct messages, reactions, custom emoji and stickers, Drive files and folders, attachments, and the previews that Orbit makes of files.',
          'Activity: notifications, and an audit log of important actions (who did what, and when) for workspace admins.',
          'Integration settings: API tokens, connected AI clients (OAuth), webhooks, chat bots, GitHub connections and the services that admins connect to the AI assistant. Their tokens and secrets are stored hashed or encrypted.',
          'Push notifications: the push subscription of your browser or the device token of your iPhone, if you turn notifications on.',
        ],
        'Direct messages and other content are stored on the server and are not end-to-end encrypted. People with administrator access to the server can technically see them; we look at content only to operate, secure or repair the service, or when the law requires it.',
      ],
    },
    {
      title: '2. Why we use the data',
      body: [
        'We use the data only to provide Orbit to you and your workspace: to sign you in, to show and sync your content, to send the notifications and emails that you expect, to keep the service secure, and to fix problems. We do not sell data, show ads, or use tracking or analytics tools.',
      ],
    },
    {
      title: '3. AI assistant',
      body: [
        'The AI assistant works only after a person connects their own ChatGPT account. When you or a routine uses it, your request and the workspace content that it reads to answer (for example tasks, docs, messages or files that you can see) are sent to OpenAI for processing. OpenAI\'s terms and privacy policy apply to that processing. The ChatGPT credential is stored encrypted on the server. The assistant\'s conversations and the memories that it keeps are stored in Orbit.',
        'If you connect an AI client of your own (for example through MCP), that client reads and changes data with your permissions, and its provider processes the data under its own terms.',
      ],
    },
    {
      title: '4. Other services that get data',
      body: [
        [
          'Email: account emails (invitations, sign-up confirmation, password recovery, email changes) go through the mail provider that we configure ([Email provider]).',
          'Push notifications: a short text (for example who wrote and the start of a message) goes through Apple Push Notification service for the iPhone app, or through your browser vendor\'s push service (for example Google, Mozilla or Apple) for browser notifications.',
          'Integrations that a workspace turns on: GitHub (issues, pull requests and comments are synced in both directions), Notion (for an import), outgoing webhooks and chat bots (message content goes to the address that an admin sets), and services that admins connect to the AI assistant (for example Shopify).',
          'Link previews: when a message contains a link, the Orbit server reads that page (an X post through the FxTwitter service, a YouTube video through YouTube) to show a preview. Your browser can then load the preview image or a video from that site.',
          'Payments: if paid plans are turned on for this server, Stripe processes them. [Remove this point if billing is off.]',
          'Hosting: the server, its files and its backups are stored at [Hosting provider and location].',
        ],
      ],
    },
    {
      title: '5. Voice input',
      body: [
        'Orbit does not record or store audio; it gets only the text. In a web browser, the browser\'s own speech recognition is used, and the browser vendor (for example Google in Chrome or Apple in Safari) can process the audio. The Windows desktop app uses a Whisper model on your computer, and the iPhone app uses speech recognition on the device, so the audio does not leave the device.',
      ],
    },
    {
      title: '6. Logs, cookies and local storage',
      body: [
        'The Orbit server logs requests (method, path, result and errors) to operate and fix the service. The web server in front of Orbit can also log IP addresses and browser information; we keep these logs for [Log retention period].',
        'Orbit uses one cookie to keep you signed in, and your browser\'s local storage for settings such as the theme. They are necessary for the service; there are no tracking cookies.',
      ],
    },
    {
      title: '7. Backups and retention',
      body: [
        'We keep your data while your account or workspace exists. Deleted tasks, projects, docs and Drive files go to the trash first and are deleted permanently after 30 days. Backups of the database and files are made regularly and kept for [Backup retention period], so deleted data can stay in a backup until that backup is removed. After an account ends, we keep its data for [Retention period after an account ends].',
      ],
    },
    {
      title: '8. Your choices and rights',
      body: [
        `You can change your profile, delete your own content, turn notifications off, disconnect your ChatGPT account from the AI assistant, and sign out other sessions in Account settings. To get a copy of your data, to correct it, or to delete your account, write to ${contact}. Depending on where you live, you can have more rights under data protection law, and you can complain to your data protection authority.`,
      ],
    },
    {
      title: '9. Changes to this policy',
      body: ['We can change this policy. The date at the top shows the current version.'],
    },
    {
      title: '10. Contact',
      body: [`Questions about privacy, or requests about your data: write to ${contact}.`],
    },
  ],
}

export const LEGAL_EFFECTIVE_DATE = '[Effective date]'
